//! Byte relay of the `stdio` mode (spec 011): what the LLM client writes goes to the application's pipe, what the
//! pipe answers goes back to the client. No secret and no MCP logic go through here, only bytes.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// Bytes relayed in each direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayStats {
    /// From `input` to the pipe; `0` if the pipe closed before `input` ended (the count is not known then).
    pub sent: u64,
    /// From the pipe to `output`.
    pub received: u64,
}

/// Relays `input` → `pipe` and `pipe` → `output` until the pipe is closed.
///
/// End of `input` closes the pipe's write side, then the remaining answers are still relayed. End of the pipe
/// flushes and closes `output` and ends the relay, whether `input` is still open or not.
pub async fn relay<I, O, P>(mut input: I, mut output: O, pipe: P) -> io::Result<RelayStats>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    P: AsyncRead + AsyncWrite + Unpin,
{
    let (mut from_pipe, mut to_pipe) = tokio::io::split(pipe);
    let upstream = async {
        let sent = tokio::io::copy(&mut input, &mut to_pipe).await?;
        to_pipe.shutdown().await?;
        Ok::<_, io::Error>(sent)
    };
    let downstream = async {
        let received = tokio::io::copy(&mut from_pipe, &mut output).await?;
        output.flush().await?;
        output.shutdown().await?;
        Ok::<_, io::Error>(received)
    };
    tokio::pin!(upstream, downstream);

    let mut sent = None;
    loop {
        tokio::select! {
            result = &mut upstream, if sent.is_none() => sent = Some(result?),
            result = &mut downstream => {
                return Ok(RelayStats { sent: sent.unwrap_or(0), received: result? });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, duplex};

    use super::*;

    // Exigence : EX-006-07
    #[tokio::test]
    async fn bytes_flow_both_ways_and_end_of_input_reaches_the_pipe() {
        let (client_in, mut client_writer) = duplex(64);
        let (mut client_reader, client_out) = duplex(64);
        let (pipe, mut app) = duplex(64);
        let relay = tokio::spawn(relay(client_in, client_out, pipe));

        client_writer.write_all(b"requete").await.unwrap();
        drop(client_writer);
        let mut received = Vec::new();
        app.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, b"requete", "end of input closes the pipe");

        app.write_all(b"reponse").await.unwrap();
        app.shutdown().await.unwrap();
        drop(app);
        let mut answer = Vec::new();
        client_reader.read_to_end(&mut answer).await.unwrap();
        assert_eq!(answer, b"reponse");
        let stats = relay.await.unwrap().unwrap();
        assert_eq!(stats, RelayStats { sent: 7, received: 7 });
    }

    // Exigence : EX-006-07
    #[tokio::test]
    async fn end_of_the_pipe_ends_the_relay_even_with_input_open() {
        let (client_in, _client_writer) = duplex(64);
        let (mut client_reader, client_out) = duplex(64);
        let (pipe, mut app) = duplex(64);
        let relay = tokio::spawn(relay(client_in, client_out, pipe));

        app.write_all(b"au revoir").await.unwrap();
        drop(app);
        let mut answer = Vec::new();
        client_reader.read_to_end(&mut answer).await.unwrap();
        assert_eq!(answer, b"au revoir");
        let stats = relay.await.unwrap().unwrap();
        assert_eq!(stats.received, 9);
    }
}
