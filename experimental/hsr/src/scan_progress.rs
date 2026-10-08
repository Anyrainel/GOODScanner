/// Native UI observer; it neither owns scanner state nor affects exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanCategory {
    Characters,
    LightCones,
    Gear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    Started,
    Progress {
        recognized: usize,
        visited: usize,
        total: Option<usize>,
    },
    Finished {
        recognized: usize,
        complete: bool,
    },
}

pub type ScanObserver = Box<dyn Fn(ScanCategory, ScanEvent) + Send>;
