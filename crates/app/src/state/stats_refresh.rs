use dioxus::prelude::*;

/// Bumped after a stats delete or restore so personal stats views refetch.
#[derive(Clone, Copy)]
pub struct StatsViewRefresh {
    generation: Signal<u64>,
}

impl StatsViewRefresh {
    pub fn provide() -> Self {
        let generation = use_signal(|| 0u64);
        let refresh = Self { generation };
        use_context_provider(|| refresh);
        refresh
    }

    pub fn current() -> Self {
        use_context()
    }

    pub fn bump(mut self) {
        self.generation += 1;
    }

    pub fn generation(self) -> u64 {
        (self.generation)()
    }
}
