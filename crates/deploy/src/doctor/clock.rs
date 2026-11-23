//! The clock, measured against the `Date` headers of the answers the other checks received: no request of its own.
//! The expiry of the tokens (spec 010) is compared to this clock with a margin of 60 s.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::report::{Finding, Status};

/// Beyond this, the gap is worth a warning: a token near its end may be refused or kept a little too long.
pub const WARNING_SKEW: Duration = Duration::from_secs(60);
/// Beyond this, the gap is a failure: the margins of the tokens no longer hold.
pub const FAILURE_SKEW: Duration = Duration::from_secs(5 * 60);

type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// What the network checks heard: the server's date and the local time it arrived, on the clock of the caller.
#[derive(Clone)]
pub struct Dates {
    seen: Arc<Mutex<Vec<(SystemTime, SystemTime)>>>,
    clock: Clock,
}

impl std::fmt::Debug for Dates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dates").finish_non_exhaustive()
    }
}

impl Default for Dates {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemTime::now))
    }
}

impl Dates {
    /// The local time of each answer is read on `clock` (the clock of the gateway, injected in its tests).
    pub fn with_clock(clock: Clock) -> Self {
        Self {
            seen: Arc::default(),
            clock,
        }
    }

    /// Keeps the `Date` header of an answer received now; a header that is not an HTTP date is ignored.
    pub fn record(&self, header: Option<&str>) {
        self.record_at(header, (self.clock)());
    }

    /// Keeps the `Date` header of an answer received at `local`.
    pub fn record_at(&self, header: Option<&str>, local: SystemTime) {
        if let Some(server) = header.and_then(http_date) {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((server, local));
        }
    }

    /// The smallest gap between the local clock and a server: the clock is said wrong only when every server
    /// disagrees with it. Positive when the local clock is ahead.
    pub fn smallest_skew(&self) -> Option<i64> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(server, local)| seconds(*local) - seconds(*server))
            .min_by_key(|skew| skew.unsigned_abs())
    }
}

fn seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// An IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`), the only form a server may send today; anything else is
/// `None`.
pub fn http_date(text: &str) -> Option<SystemTime> {
    let rest = text.trim().split_once(", ")?.1;
    let parts: Vec<&str> = rest.split(' ').collect();
    let [day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    let number = |s: &str, len: usize| -> Option<i64> {
        (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let day = number(day, 2)?;
    let month = i64::try_from(MONTHS.iter().position(|m| m == month)? + 1).ok()?;
    let year = number(year, 4)?;
    let clock: Vec<&str> = time.split(':').collect();
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    let (hour, minute, second) = (number(hour, 2)?, number(minute, 2)?, number(second, 2)?);
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let secs = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    let secs = u64::try_from(secs).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

/// The status of a gap of `skew` seconds.
pub fn skew_status(skew: i64) -> Status {
    let gap = skew.unsigned_abs();
    if gap > FAILURE_SKEW.as_secs() {
        Status::Failure
    } else if gap > WARNING_SKEW.as_secs() {
        Status::Warning
    } else {
        Status::Ok
    }
}

fn duration_text(seconds: u64) -> String {
    match seconds {
        s if s < 120 => format!("{s} s"),
        s if s < 7200 => format!("{} min", s / 60),
        s if s < 172_800 => format!("{} h", s / 3600),
        s => format!("{} jours", s / 86_400),
    }
}

/// « Horloge »: run once the network checks are done.
#[derive(Debug, Clone)]
pub struct ClockCheck {
    pub dates: Dates,
}

impl ClockCheck {
    pub fn run(&self) -> Vec<Finding> {
        let Some(skew) = self.dates.smallest_skew() else {
            return vec![Finding::ok(
                "écart non mesuré : aucun serveur n'a donné son heure (aucun serveur contacté ou joignable)",
            )];
        };
        let gap = duration_text(skew.unsigned_abs());
        let direction = if skew > 0 { "en avance" } else { "en retard" };
        let fix = "Réglez l'heure de Windows : Paramètres, « Heure et langue », « Date et heure », activez « Régler \
                   l'heure automatiquement » puis « Synchroniser maintenant ». Les échéances des connexions en \
                   dépendent.";
        match skew_status(skew) {
            Status::Ok => vec![Finding::ok(format!(
                "à l'heure des serveurs (écart de {} s au plus)",
                skew.unsigned_abs()
            ))],
            Status::Warning => vec![Finding::warning(
                format!("l'horloge de ce poste est {direction} de {gap} sur celle des serveurs"),
                fix,
            )],
            Status::Failure => vec![Finding::failure(
                format!(
                    "l'horloge de ce poste est {direction} de {gap} sur celle des serveurs : les connexions \
                     peuvent être refusées ou expirer trop tôt"
                ),
                fix,
            )],
        }
    }
}
