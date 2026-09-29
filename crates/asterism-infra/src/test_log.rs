//! Test-only capture of what this crate logs, for the tests that assert
//! a diagnosis was emitted rather than only that nothing failed.

/// Collects the `event` field of everything emitted while it is the
/// thread's subscriber.
///
/// The field and not the rendered line: `event` is the name a
/// diagnosis is searched for by, so asserting on it is asserting on
/// the thing operators use, and it cannot pass because some other
/// part of a message happened to contain the string. The cost is one
/// `Visit` impl that keeps a single key.
#[derive(Clone, Default)]
pub(crate) struct EventNames(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl EventNames {
    /// Every `event` name seen so far, in emission order.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventNames {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Name<'a>(&'a mut Option<String>);
        impl tracing::field::Visit for Name<'_> {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "event" {
                    *self.0 = Some(value.to_string());
                }
            }
            // Every other field is somebody's id and none of them is
            // what this is watching.
            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }
        let mut name = None;
        event.record(&mut Name(&mut name));
        if let Some(name) = name {
            self.0.lock().unwrap().push(name);
        }
    }
}
