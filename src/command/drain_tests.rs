use std::time::Duration;

use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};

#[test]
fn open_pipe_holders_cannot_keep_the_output_reader_past_its_deadline() -> anyhow::Result<()> {
    let (out_reader, out_holder) = UnixStream::pair().test()?;
    let (err_reader, err_holder) = UnixStream::pair().test()?;
    let (cancel, receiver) = UnixStream::pair().test()?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(100))
        .test()?;
    let error = capture(
        Reader::new(out_reader.into()).test()?,
        Reader::new(err_reader.into()).test()?,
        &receiver,
        deadline,
    )
    .test_err()?;
    assert!(error.to_string().contains("deadline expired"));
    // These holders deliberately remain open until capture has returned.
    drop((out_holder, err_holder, cancel));
    Ok(())
}

#[test]
fn dropping_capture_cancels_and_joins_without_waiting_for_pipe_eof() -> anyhow::Result<()> {
    let (out_reader, out_holder) = UnixStream::pair().test()?;
    let (err_reader, err_holder) = UnixStream::pair().test()?;
    let (cancel, receiver) = UnixStream::pair().test()?;
    let deadline = Instant::now().checked_add(Duration::from_secs(10)).test()?;
    let stdout = Reader::new(out_reader.into()).test()?;
    let stderr = Reader::new(err_reader.into()).test()?;
    let (send, completion) = std::sync::mpsc::channel();
    let worker = thread::Builder::new()
        .spawn(move || {
            let result = capture(stdout, stderr, &receiver, deadline);
            send.send(result.as_ref().err().map(ToString::to_string))
                .test()?;
            result
        })
        .test()?;
    drop(Drain {
        cancel: Some(cancel),
        worker: Some(worker),
    });
    let error = completion.recv().test()?.test()?;
    assert!(error.contains("capture cancelled"));
    drop((out_holder, err_holder));
    Ok(())
}
