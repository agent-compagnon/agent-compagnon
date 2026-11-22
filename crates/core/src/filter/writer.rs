//! `tracing` writer that passes every formatted record through the filter before writing it: defence in depth for
//! the logs, behind the filtering of the values themselves.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::Arc;

use tracing_subscriber::fmt::MakeWriter;
use zeroize::Zeroizing;

use super::OutputFilter;

/// Written instead of a record when the filter is unavailable.
pub const UNFILTERED_RECORD: &str = "[ligne de journal retirée : filtre de sortie indisponible]\n";

const INITIAL_CAPACITY: usize = 1024;

/// Wraps a `MakeWriter`: what `tracing_subscriber::fmt` writes is buffered, filtered with `OutputFilter::filter_text`
/// and only then handed to the inner writer, when the record is complete (writer dropped or flushed).
pub struct FilteringMakeWriter<M> {
    inner: M,
    filter: Arc<OutputFilter>,
}

impl<M> FilteringMakeWriter<M> {
    pub fn new(inner: M, filter: Arc<OutputFilter>) -> Self {
        Self { inner, filter }
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for FilteringMakeWriter<M> {
    type Writer = FilteringWriter<'a, M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        FilteringWriter {
            inner: self.inner.make_writer(),
            filter: &self.filter,
            buf: Zeroizing::new(Vec::with_capacity(INITIAL_CAPACITY)),
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        FilteringWriter {
            inner: self.inner.make_writer_for(meta),
            filter: &self.filter,
            buf: Zeroizing::new(Vec::with_capacity(INITIAL_CAPACITY)),
        }
    }
}

pub struct FilteringWriter<'a, W: Write> {
    inner: W,
    filter: &'a OutputFilter,
    /// Unfiltered bytes: wiped on drop, and never reallocated in place (see `write`).
    buf: Zeroizing<Vec<u8>>,
}

impl<W: Write> FilteringWriter<'_, W> {
    fn emit(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let result = {
            let text = String::from_utf8_lossy(&self.buf);
            let result = match self.filter.filter_text(&text) {
                Ok((filtered, _)) => self.inner.write_all(filtered.as_bytes()),
                Err(_) => self.inner.write_all(UNFILTERED_RECORD.as_bytes()),
            };
            if let Cow::Owned(copy) = text {
                drop(Zeroizing::new(copy));
            }
            result
        };
        self.buf.clear();
        result
    }
}

impl<W: Write> Write for FilteringWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let needed = self.buf.len() + data.len();
        if needed > self.buf.capacity() {
            // Grow into a fresh buffer so that the old one is wiped instead of being left behind by `realloc`.
            let mut grown = Zeroizing::new(Vec::with_capacity(needed.next_power_of_two()));
            grown.extend_from_slice(&self.buf);
            self.buf = grown;
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.emit()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for FilteringWriter<'_, W> {
    fn drop(&mut self) {
        // Nowhere to report an error from `drop`: the record is lost, which is the safe outcome.
        let _ = self.emit();
    }
}
