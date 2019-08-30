use std::collections::HashSet;

use crate::config;

pub trait Filter {
    fn after_wp_refresh(&mut self, _: &[&str]) {}
    fn is_filtered(&mut self, wp: &str) -> bool;
}

#[derive(Default)]
pub struct LastShown {
    last: HashSet<String>,
}

impl Filter for LastShown {
    fn after_wp_refresh(&mut self, new_wps: &[&str]) {
        self.last.clear();
        for wp in new_wps {
            self.last.insert(wp.to_string());
        }
    }

    fn is_filtered(&mut self, wp: &str) -> bool {
        self.last.contains(wp)
    }
}

impl From<config::Filter> for Box<dyn Filter> {
    fn from(other: config::Filter) -> Self {
        match other {
            config::Filter::LastShown => Box::new(LastShown::default()),
        }
    }
}
