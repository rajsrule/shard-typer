use crate::{settings::NewlineMode, text::PreparedText, timing::TimingConfig};
use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Countdown,
    WaitingForKeys,
    Typing,
    Paused,
    Completed,
    Error,
}
impl SessionState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Countdown | Self::WaitingForKeys | Self::Typing | Self::Paused
        )
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub state: SessionState,
    pub position: usize,
    pub total: usize,
    pub countdown: f64,
    pub elapsed: f64,
    pub message: String,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: SessionState::Idle,
            position: 0,
            total: 0,
            countdown: 0.,
            elapsed: 0.,
            message: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TypingEvent {
    pub index: usize,
    pub planned_ms: f64,
    pub actual_ms: Option<f64>,
}
#[derive(Clone, Debug)]
pub enum WorkerEvent {
    Status(Snapshot),
    Character(TypingEvent),
}
pub enum Command {
    Start(Arc<PreparedText>, NewlineMode, Duration),
    Resume(Duration),
    Pause,
    Stop,
    Timing(TimingConfig),
    Shutdown,
}

pub trait Clock {
    fn now(&self) -> Duration;
}
pub trait RandomSource {
    fn delay_ms(&mut self, config: &TimingConfig) -> f64;
}
pub trait InputAdapter {
    fn foreground(&self) -> Option<u64>;
    fn is_own_window(&self, target: u64) -> bool;
    fn modifiers_down(&self) -> bool;
    fn send(&mut self, text: &str, newline: NewlineMode) -> Result<(), String>;
}

pub struct RealClock(Instant);
impl Default for RealClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}
pub struct RealRandom(rand::rngs::ThreadRng);
impl Default for RealRandom {
    fn default() -> Self {
        Self(rand::rng())
    }
}
impl RandomSource for RealRandom {
    fn delay_ms(&mut self, c: &TimingConfig) -> f64 {
        c.sample(&mut self.0)
    }
}

pub struct Session<C, R, I> {
    clock: C,
    random: R,
    input: I,
    config: TimingConfig,
    pub snapshot: Snapshot,
    text: Arc<PreparedText>,
    newline: NewlineMode,
    target: Option<u64>,
    due: Option<Duration>,
    planned_ms: f64,
    countdown_end: Duration,
    last_submission: Option<Duration>,
    active_since: Option<Duration>,
    active_time: Duration,
}

impl<C: Clock, R: RandomSource, I: InputAdapter> Session<C, R, I> {
    pub fn new(clock: C, random: R, input: I, config: TimingConfig) -> Self {
        Self {
            clock,
            random,
            input,
            config,
            snapshot: Snapshot::default(),
            text: Arc::new(PreparedText::new("")),
            newline: NewlineMode::Enter,
            target: None,
            due: None,
            planned_ms: 0.,
            countdown_end: Duration::ZERO,
            last_submission: None,
            active_since: None,
            active_time: Duration::ZERO,
        }
    }
    pub fn command(&mut self, command: Command) {
        match command {
            Command::Start(text, newline, delay) => {
                if self.snapshot.state.active() {
                    return;
                }
                self.text = text;
                self.newline = newline;
                self.active_time = Duration::ZERO;
                self.snapshot = Snapshot {
                    total: self.text.len(),
                    ..Default::default()
                };
                if self.text.is_empty() {
                    self.fail("Add some text before starting.".into());
                } else {
                    self.begin(delay);
                }
            }
            Command::Resume(delay) if self.snapshot.state == SessionState::Paused => {
                self.begin(delay)
            }
            Command::Pause if self.snapshot.state.active() => self.pause("Paused"),
            Command::Stop => {
                self.end_active();
                self.snapshot = Snapshot::default();
                self.target = None;
                self.due = None;
                self.last_submission = None;
                self.active_time = Duration::ZERO;
            }
            Command::Timing(c) if c.validate().is_ok() => {
                self.config = c;
            }
            _ => {}
        }
    }
    fn begin(&mut self, delay: Duration) {
        self.target = None;
        self.due = None;
        self.last_submission = None;
        self.countdown_end = self.clock.now() + delay;
        self.snapshot.countdown = delay.as_secs_f64();
        self.snapshot.message.clear();
        self.snapshot.state = if delay.is_zero() {
            SessionState::WaitingForKeys
        } else {
            SessionState::Countdown
        };
    }
    fn end_active(&mut self) {
        if let Some(since) = self.active_since.take() {
            self.active_time += self.clock.now().saturating_sub(since);
        }
        self.snapshot.elapsed = self.active_time.as_secs_f64();
    }
    fn pause(&mut self, message: &str) {
        self.end_active();
        self.snapshot.state = SessionState::Paused;
        self.snapshot.message = message.into();
        self.due = None;
        self.last_submission = None;
    }
    fn fail(&mut self, message: String) {
        self.end_active();
        self.snapshot.state = SessionState::Error;
        self.snapshot.message = message;
        self.due = None;
        self.last_submission = None;
    }
    fn schedule(&mut self) {
        self.planned_ms = self.random.delay_ms(&self.config);
        self.due = Some(self.clock.now() + Duration::from_secs_f64(self.planned_ms / 1000.));
    }
    pub fn tick(&mut self) -> Option<TypingEvent> {
        let now = self.clock.now();
        if let Some(since) = self.active_since {
            self.snapshot.elapsed = (self.active_time + now.saturating_sub(since)).as_secs_f64();
        }
        if self.snapshot.state == SessionState::Countdown {
            self.snapshot.countdown = self.countdown_end.saturating_sub(now).as_secs_f64();
            if now < self.countdown_end {
                return None;
            }
            self.snapshot.state = SessionState::WaitingForKeys;
        }
        if self.snapshot.state == SessionState::WaitingForKeys {
            if self.input.modifiers_down() {
                self.snapshot.message = "Release the hotkey keys…".into();
                return None;
            }
            let Some(target) = self
                .input
                .foreground()
                .filter(|t| !self.input.is_own_window(*t))
            else {
                self.pause("Select a destination, then press the hotkey to resume.");
                return None;
            };
            self.target = Some(target);
            self.active_since = Some(now);
            self.snapshot.state = SessionState::Typing;
            self.snapshot.message.clear();
            self.schedule();
        }
        if self.snapshot.state != SessionState::Typing {
            return None;
        }
        if self.input.foreground() != self.target {
            self.pause("Destination lost focus. Select it and press the hotkey to resume.");
            return None;
        }
        if self.due.is_some_and(|due| now < due) {
            return None;
        }
        let index = self.snapshot.position;
        if let Some(grapheme) = self.text.grapheme(index) {
            if let Err(e) = self.input.send(grapheme, self.newline) {
                self.fail(e);
                return None;
            }
        } else {
            self.fail("The session text is no longer available.".into());
            return None;
        }
        let submitted = self.clock.now();
        let event = TypingEvent {
            index,
            planned_ms: self.planned_ms,
            actual_ms: self
                .last_submission
                .map(|last| submitted.saturating_sub(last).as_secs_f64() * 1000.),
        };
        self.last_submission = Some(submitted);
        self.snapshot.position += 1;
        if self.snapshot.position == self.text.len() {
            self.end_active();
            self.snapshot.state = SessionState::Completed;
            self.snapshot.message = "All words delivered.".into();
            self.due = None;
        } else {
            self.schedule();
        }
        Some(event)
    }
}

pub struct Worker {
    pub tx: mpsc::Sender<Command>,
    pub rx: mpsc::Receiver<WorkerEvent>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn spawn(config: TimingConfig, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut session = Session::new(
                RealClock::default(),
                RealRandom::default(),
                crate::platform::PlatformInput,
                config,
            );
            let mut published = Instant::now();
            let mut previous_state = SessionState::Idle;
            loop {
                let active = matches!(
                    session.snapshot.state,
                    SessionState::Countdown | SessionState::WaitingForKeys | SessionState::Typing
                );
                let command = if active {
                    match commands.recv_timeout(Duration::from_millis(10)) {
                        Ok(c) => Some(c),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(_) => break,
                    }
                } else {
                    match commands.recv() {
                        Ok(c) => Some(c),
                        Err(_) => break,
                    }
                };
                let commanded = command.is_some();
                if let Some(command) = command {
                    if matches!(command, Command::Shutdown) {
                        break;
                    }
                    session.command(command);
                }
                if let Some(event) = session.tick() {
                    if events.send(WorkerEvent::Character(event)).is_err() {
                        break;
                    }
                    (wake)();
                }
                if commanded
                    || session.snapshot.state != previous_state
                    || published.elapsed() >= Duration::from_millis(100)
                {
                    previous_state = session.snapshot.state;
                    published = Instant::now();
                    if events
                        .send(WorkerEvent::Status(session.snapshot.clone()))
                        .is_err()
                    {
                        break;
                    }
                    (wake)();
                }
            }
        });
        Self {
            tx,
            rx,
            handle: Some(handle),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    #[derive(Clone)]
    struct FakeClock(Rc<Cell<Duration>>);
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
    }
    struct FakeRandom;
    impl RandomSource for FakeRandom {
        fn delay_ms(&mut self, c: &TimingConfig) -> f64 {
            c.center_ms
        }
    }
    #[derive(Clone)]
    struct FakeInput {
        target: Rc<Cell<Option<u64>>>,
        modifiers: Rc<Cell<bool>>,
        fail: Rc<Cell<bool>>,
        sent: Rc<RefCell<Vec<String>>>,
    }
    impl InputAdapter for FakeInput {
        fn foreground(&self) -> Option<u64> {
            self.target.get()
        }
        fn is_own_window(&self, t: u64) -> bool {
            t == 99
        }
        fn modifiers_down(&self) -> bool {
            self.modifiers.get()
        }
        fn send(&mut self, s: &str, _: NewlineMode) -> Result<(), String> {
            if self.fail.get() {
                Err("partial send".into())
            } else {
                self.sent.borrow_mut().push(s.into());
                Ok(())
            }
        }
    }
    fn fixture() -> (
        Session<FakeClock, FakeRandom, FakeInput>,
        FakeClock,
        FakeInput,
    ) {
        let clock = FakeClock(Rc::new(Cell::new(Duration::ZERO)));
        let input = FakeInput {
            target: Rc::new(Cell::new(Some(1))),
            modifiers: Rc::new(Cell::new(false)),
            fail: Rc::new(Cell::new(false)),
            sent: Rc::new(RefCell::new(vec![])),
        };
        (
            Session::new(
                clock.clone(),
                FakeRandom,
                input.clone(),
                TimingConfig::default(),
            ),
            clock,
            input,
        )
    }
    fn advance(clock: &FakeClock, ms: u64) {
        clock.0.set(clock.now() + Duration::from_millis(ms));
    }
    #[test]
    fn countdown_cancellation_never_sends() {
        let (mut s, c, i) = fixture();
        s.command(Command::Start(
            Arc::new(PreparedText::new("abc")),
            NewlineMode::Enter,
            Duration::from_secs(5),
        ));
        advance(&c, 4999);
        s.tick();
        assert_eq!(s.snapshot.state, SessionState::Countdown);
        s.command(Command::Stop);
        advance(&c, 5000);
        s.tick();
        assert!(i.sent.borrow().is_empty());
    }
    #[test]
    fn pause_focus_resume_and_completion_preserve_position() {
        let (mut s, c, i) = fixture();
        s.command(Command::Start(
            Arc::new(PreparedText::new("a👩‍💻b")),
            NewlineMode::Enter,
            Duration::ZERO,
        ));
        s.tick();
        advance(&c, 250);
        assert!(s.tick().unwrap().actual_ms.is_none());
        i.target.set(Some(2));
        s.tick();
        assert_eq!(s.snapshot.state, SessionState::Paused);
        assert_eq!(s.snapshot.position, 1);
        advance(&c, 5000);
        s.command(Command::Resume(Duration::ZERO));
        s.tick();
        advance(&c, 250);
        assert!(s.tick().unwrap().actual_ms.is_none());
        advance(&c, 250);
        assert_eq!(s.tick().unwrap().actual_ms, Some(250.));
        assert_eq!(s.snapshot.state, SessionState::Completed);
        assert_eq!(*i.sent.borrow(), vec!["a", "👩‍💻", "b"]);
        assert!((s.snapshot.elapsed - 0.75).abs() < 1e-8);
    }
    #[test]
    fn modifiers_own_window_and_failed_input() {
        let (mut s, c, i) = fixture();
        i.modifiers.set(true);
        s.command(Command::Start(
            Arc::new(PreparedText::new("a")),
            NewlineMode::Enter,
            Duration::ZERO,
        ));
        advance(&c, 500);
        s.tick();
        assert_eq!(s.snapshot.state, SessionState::WaitingForKeys);
        i.modifiers.set(false);
        i.target.set(Some(99));
        s.tick();
        assert_eq!(s.snapshot.state, SessionState::Paused);
        i.target.set(Some(1));
        s.command(Command::Resume(Duration::ZERO));
        s.tick();
        i.fail.set(true);
        advance(&c, 250);
        s.tick();
        assert_eq!(s.snapshot.state, SessionState::Error);
        assert_eq!(s.snapshot.position, 0);
        s.tick();
        assert!(i.sent.borrow().is_empty());
    }
    #[test]
    fn live_timing_applies_after_already_scheduled_character() {
        let (mut s, c, _) = fixture();
        s.command(Command::Start(
            Arc::new(PreparedText::new("abc")),
            NewlineMode::Enter,
            Duration::ZERO,
        ));
        s.tick();
        s.command(Command::Timing(TimingConfig {
            center_ms: 100.,
            ..Default::default()
        }));
        advance(&c, 250);
        assert_eq!(s.tick().unwrap().planned_ms, 250.);
        advance(&c, 100);
        assert_eq!(s.tick().unwrap().planned_ms, 100.);
        s.command(Command::Timing(TimingConfig {
            min_ms: -1.,
            ..Default::default()
        }));
        advance(&c, 100);
        assert_eq!(s.tick().unwrap().planned_ms, 100.);
    }
}
