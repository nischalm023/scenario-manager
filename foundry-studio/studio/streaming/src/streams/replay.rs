//! Within `streams`, a `replay` is what a viewer attaching to a stream is sent before the live messages, each message
//! encoded once and shared: the stream's `started`, its newest `progress`, and its last word, `ended` or
//! `runner-lost`.

use axum::extract::ws::Utf8Bytes;

#[derive(Debug, Default)]
pub struct Replay {
    started: Option<Utf8Bytes>,
    progress: Option<Utf8Bytes>,
    last_word: Option<Utf8Bytes>,
}

impl Replay {
    pub fn started(&mut self, message: Utf8Bytes) {
        self.started = Some(message);
    }

    pub fn progress(&mut self, message: Utf8Bytes) {
        self.progress = Some(message);
    }

    /// The stream's last word, or none when a lost runner came back.
    pub fn last_word(&mut self, message: Option<Utf8Bytes>) {
        self.last_word = message;
    }

    /// Everything held let go, as a forgotten stream does.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The messages in the order a viewer is sent them.
    pub fn messages(&self) -> Vec<Utf8Bytes> {
        self.started.iter().chain(&self.progress).chain(&self.last_word).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::ws::Utf8Bytes;

    use super::Replay;

    fn text(words: &str) -> Utf8Bytes {
        words.into()
    }

    /// Whatever order they come in, a viewer is sent `started`, the newest progress and the last word, and nothing
    /// once the replay is let go.
    #[test]
    fn the_newest_of_each_is_held_in_the_order_a_viewer_is_sent_them() {
        let mut replay = Replay::default();
        replay.last_word(Some(text("runner-lost")));
        replay.progress(text("progress at 10 s"));
        replay.started(text("started"));
        replay.progress(text("progress at 20 s"));
        replay.last_word(None);
        replay.last_word(Some(text("ended")));
        let sent: Vec<String> = replay.messages().iter().map(ToString::to_string).collect();
        assert_eq!(sent, ["started", "progress at 20 s", "ended"]);
        replay.clear();
        assert!(replay.messages().is_empty());
    }
}
