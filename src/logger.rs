use log::Record;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

thread_local! {
    static REQUEST_ID: Cell<Option<Uuid>> = Cell::new(None);
}

fn format_record_to_buf(
    f: &mut env_logger::fmt::Formatter,
    record: &Record,
) -> std::io::Result<()> {
    use std::io::Write;
    let rid = REQUEST_ID.with(|rid| {
        if let Some(uuid) = rid.get() {
            format!(" request_id={}", uuid)
        } else {
            String::from("")
        }
    });
    writeln!(
        f,
        "[{time} {level:<5} {module_path} {file}:{line}]{request_id} {record}",
        time = f.timestamp_millis(),
        request_id = rid,
        level = record.level(),
        module_path = record.module_path().unwrap_or(""),
        file = record.file().unwrap_or("???"),
        line = record.line().map(|l| l as i64).unwrap_or(-1),
        record = record.args(),
    )
}

pub fn init() {
    eprintln!("setting logger");
    log::set_boxed_logger(Box::new(
        env_logger::Builder::from_default_env()
            .format(format_record_to_buf)
            .build(),
    ))
    .unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    log::error!("initialized logging infra");
}

pub struct LogFuture<F> {
    uuid: Uuid,
    future: F,
}

impl<F: Future> Future for LogFuture<F> {
    type Output = F::Output;
    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        unsafe {
            let self_ = self.get_unchecked_mut();
            REQUEST_ID.with(|thread| {
                let uuid = self_.uuid;
                thread.set(Some(uuid));
                let fut = Pin::new_unchecked(&mut self_.future);
                let ret = fut.poll(cx);
                thread.set(None);
                ret
            })
        }
    }
}

impl<F> LogFuture<F> {
    pub fn new(uuid: Uuid, future: F) -> Self {
        Self { uuid, future }
    }
}
