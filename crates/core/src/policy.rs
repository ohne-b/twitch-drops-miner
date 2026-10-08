use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};

use crate::{config::fold, domain::Drop};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreReason {
    Keyword(String),
    Precondition(String),
}

impl IgnoreReason {
    pub fn kind(&self) -> &str {
        match self {
            Self::Keyword(_) => "keyword",
            Self::Precondition(_) => "precondition",
        }
    }
    pub fn keyword(&self) -> Option<&str> {
        match self {
            Self::Keyword(v) => Some(v),
            _ => None,
        }
    }
    pub fn precondition(&self) -> Option<&str> {
        match self {
            Self::Precondition(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct DropPolicy {
    pub reasons: HashMap<String, IgnoreReason>,
    pub mineable: HashSet<String>,
    remaining: HashMap<String, u32>,
    prerequisite_order: Vec<String>,
}

pub struct WatchPriority {
    pub deadline: DateTime<Utc>,
    pub targets: Vec<String>,
}

impl DropPolicy {
    pub fn evaluate(drops: &[Drop], keywords: &[String]) -> Self {
        Self::for_targets(drops, keywords, |_| true)
    }

    pub fn for_targets(
        drops: &[Drop],
        keywords: &[String],
        target: impl Fn(&Drop) -> bool,
    ) -> Self {
        let by_id: HashMap<_, _> = drops.iter().map(|d| (d.id.as_str(), d)).collect();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for drop in drops {
            for id in &drop.prerequisites {
                dependents.entry(id).or_default().push(&drop.id);
            }
        }
        let mut result = Self::default();
        let mut ignored = VecDeque::new();
        for drop in drops.iter().filter(|d| !d.claimed) {
            let name = fold(&drop.name);
            if let Some(keyword) = keywords
                .iter()
                .find(|word| !word.is_empty() && name.contains(&fold(word)))
            {
                result
                    .reasons
                    .insert(drop.id.clone(), IgnoreReason::Keyword(keyword.clone()));
                ignored.push_back(drop.id.as_str());
            }
        }
        while let Some(id) = ignored.pop_front() {
            for &child in dependents.get(id).into_iter().flatten() {
                if !by_id[child].claimed && !result.reasons.contains_key(child) {
                    result.reasons.insert(
                        child.to_owned(),
                        IgnoreReason::Precondition(by_id[id].name.clone()),
                    );
                    ignored.push_back(child);
                }
            }
        }

        // Resolve the dependency graph without recursion. Missing prerequisites and
        // cycles stay unresolved; a claimed prerequisite always satisfies its branch.
        let mut unresolved: HashMap<&str, usize> = drops
            .iter()
            .map(|d| (d.id.as_str(), d.prerequisites.len()))
            .collect();
        let mut ready: VecDeque<&str> = drops
            .iter()
            .filter(|d| d.claimed || d.prerequisites.is_empty())
            .map(|d| d.id.as_str())
            .collect();
        let mut valid = HashSet::new();
        while let Some(id) = ready.pop_front() {
            let drop = by_id[id];
            if valid.contains(id)
                || !drop.claimed && (!drop.watch_reward() || result.reasons.contains_key(id))
            {
                continue;
            }
            valid.insert(id);
            result.prerequisite_order.push(id.to_owned());
            let remaining = if drop.claimed {
                0
            } else {
                drop.remaining_minutes().saturating_add(
                    drop.prerequisites
                        .iter()
                        .filter_map(|p| result.remaining.get(p))
                        .copied()
                        .max()
                        .unwrap_or(0),
                )
            };
            result.remaining.insert(id.to_owned(), remaining);
            for child in dependents.get(id).into_iter().flatten() {
                let count = unresolved.get_mut(child).expect("known dependent");
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push_back(child);
                }
            }
        }
        let mut useful: Vec<&str> = drops
            .iter()
            .filter(|d| {
                !d.claimed && !d.benefits.is_empty() && valid.contains(d.id.as_str()) && target(d)
            })
            .map(|d| d.id.as_str())
            .collect();
        while let Some(id) = useful.pop() {
            let drop = by_id[id];
            if drop.claimed || !result.mineable.insert(id.to_owned()) {
                continue;
            }
            useful.extend(drop.prerequisites.iter().map(String::as_str));
        }
        result
    }

    pub fn watch_deadlines(
        &self,
        drops: &[Drop],
        now: DateTime<Utc>,
        target_deadline: impl Fn(&Drop) -> Option<DateTime<Utc>>,
    ) -> HashMap<String, WatchPriority> {
        let by_id: HashMap<_, _> = drops.iter().map(|d| (d.id.as_str(), d)).collect();
        let mut available = HashMap::new();
        // Reuse the resolved graph. An expired unwatched prerequisite, or one
        // starting after its dependent ends, must not boost an unrelated reward.
        // This checks timing reachability, not whether enough watch time remains.
        for id in &self.prerequisite_order {
            let drop = by_id[id.as_str()];
            let starts = if drop.claimed || drop.confirmed_minutes >= drop.required_minutes {
                Some(now)
            } else {
                drop.prerequisites
                    .iter()
                    .try_fold(now.max(drop.starts_at), |start, id| {
                        available.get(id.as_str()).map(|at| start.max(*at))
                    })
                    .filter(|start| *start < drop.ends_at)
            };
            if let Some(start) = starts {
                available.insert(drop.id.as_str(), start);
            }
        }
        let mut deadlines = HashMap::new();
        for drop in drops {
            if self.mineable.contains(&drop.id)
                && drop.confirmed_minutes < drop.required_minutes
                && let Some(end) = target_deadline(drop)
                && available
                    .get(drop.id.as_str())
                    .is_some_and(|start| *start < end)
            {
                deadlines.insert(
                    drop.id.clone(),
                    WatchPriority {
                        deadline: end,
                        targets: vec![drop.id.clone()],
                    },
                );
            }
        }
        // Dependents come first here: propagate the earliest target deadline
        // through shared prerequisites once, without recursion or per-target walks.
        for id in self.prerequisite_order.iter().rev() {
            if let Some(priority) = deadlines.get(id) {
                let end = priority.deadline;
                let targets = priority.targets.clone();
                for parent in &by_id[id.as_str()].prerequisites {
                    let drop = by_id[parent.as_str()];
                    if drop.claimed || drop.confirmed_minutes >= drop.required_minutes {
                        continue;
                    }
                    let end = end.min(drop.ends_at);
                    deadlines
                        .entry(parent.clone())
                        .and_modify(|priority| {
                            if end < priority.deadline {
                                priority.deadline = end;
                                priority.targets = targets.clone();
                            } else if end == priority.deadline {
                                for target in &targets {
                                    if !priority.targets.contains(target) {
                                        priority.targets.push(target.clone());
                                    }
                                }
                            }
                        })
                        .or_insert_with(|| WatchPriority {
                            deadline: end,
                            targets: targets.clone(),
                        });
                }
            }
        }
        deadlines
    }

    pub fn remaining_minutes(&self) -> u32 {
        self.mineable
            .iter()
            .filter_map(|id| self.remaining.get(id))
            .copied()
            .max()
            .unwrap_or(0)
    }
}
