//! Sewing order: which object follows which, and where each is entered and left.
//!
//! The algorithms here work on [`Item`]s, an abstraction of sewn objects: the thread colour an
//! object starts and ends in and the [`Way`]s it can be sewn (forwards, backwards, a closed
//! outline from another stitch point), each with its entry, its exit and what sewing it costs
//! inside. [`crate::sequence`] builds them from a document. Everything is pure and bounded:
//! [`closest_join`] is a dynamic programme over at most [`MAX_WAYS`] ways per object.

use std::collections::HashSet;
use std::hash::Hash;

use crate::Pt;

/// The most ways an object is offered to [`closest_join`]: forwards, backwards and rotations of a
/// closed outline sampled evenly around it.
pub const MAX_WAYS: usize = 50;

/// One way to sew an object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Way {
    /// Where the needle first goes down.
    pub entry: Pt,
    /// Where the object's last stitch is.
    pub exit: Pt,
    /// Trims between the object's own runs, sewn this way.
    pub trims: usize,
    /// Travel between the object's own runs, sewn this way, in plan units.
    pub travel: f64,
}

/// A sewn object, as the sequencing algorithms see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The thread it starts in.
    pub first_rgb: [u8; 3],
    /// The thread it ends in (a filled and stroked object may change thread inside).
    pub last_rgb: [u8; 3],
    /// Thread changes inside the object.
    pub changes: usize,
    /// The ways it can be sewn; never empty for an object that sews anything. One way: the object
    /// is pinned (its entry or exit was set, or it keeps an opened stitch file's needle points).
    pub ways: Vec<Way>,
}

/// What a sewing order costs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cost {
    /// Trims: between runs of one thread that do not start where the last one ended.
    pub trims: usize,
    /// Travel: the distance moved without sewing, in plan units, from the design start (when set)
    /// to the design end (when set).
    pub travel: f64,
    /// Thread changes.
    pub changes: usize,
}

/// Where the design starts and ends, and how near a run must start to be joined without a trim.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ends {
    pub start: Option<Pt>,
    pub end: Option<Pt>,
    /// Runs that start within this distance (plan units) of the last stitch are joined.
    pub min_stitch: i32,
}

/// The cost of entering `item` by `way` after an object that ended at `prev` (its exit and its
/// thread); `None`: `item` is the first.
fn join(prev: Option<(Pt, [u8; 3])>, item: &Item, way: &Way, ends: &Ends) -> Cost {
    let mut c = Cost { trims: way.trims, travel: way.travel, changes: item.changes };
    match prev {
        None => c.travel += ends.start.map_or(0.0, |s| s.dist(way.entry)),
        Some((at, rgb)) => {
            let d = at.dist(way.entry);
            c.travel += d;
            if rgb != item.first_rgb {
                c.changes += 1;
            } else if d > f64::from(ends.min_stitch) {
                c.trims += 1;
            }
        }
    }
    c
}

fn add(a: Cost, b: Cost) -> Cost {
    Cost { trims: a.trims.saturating_add(b.trims), travel: a.travel + b.travel, changes: a.changes.saturating_add(b.changes) }
}

/// The way `ways[i]` of item `i` (the first way when out of range); `None` when it has none.
fn way_of<'a>(items: &'a [Item], ways: &[usize], i: usize) -> Option<&'a Way> {
    let item = items.get(i)?;
    item.ways.get(ways.get(i).copied().unwrap_or(0)).or_else(|| item.ways.first())
}

/// What sewing `items` in order, each by way `ways[i]`, costs.
pub fn cost(items: &[Item], ways: &[usize], ends: &Ends) -> Cost {
    let mut total = Cost::default();
    let mut prev: Option<(Pt, [u8; 3])> = None;
    for (i, item) in items.iter().enumerate() {
        let Some(w) = way_of(items, ways, i) else { continue };
        total = add(total, join(prev, item, w, ends));
        prev = Some((w.exit, item.last_rgb));
    }
    if let (Some((at, _)), Some(end)) = (prev, ends.end) {
        total.travel += at.dist(end);
    }
    total
}

/// The thread changes of sewing `items` in `order` (indices into `items`; others are skipped).
pub fn colour_changes(items: &[Item], order: &[usize]) -> usize {
    let mut n = 0usize;
    let mut prev: Option<[u8; 3]> = None;
    for item in order.iter().filter_map(|&i| items.get(i)) {
        n = n.saturating_add(item.changes);
        if prev.is_some_and(|p| p != item.first_rgb) {
            n = n.saturating_add(1);
        }
        prev = Some(item.last_rgb);
    }
    n
}

/// Is `a` better than `b`, by `key` order: trims first, or travel first?
fn better(a: &Cost, b: &Cost, trims_first: bool) -> bool {
    let (a1, a2, b1, b2) =
        if trims_first { (a.trims as f64, a.travel, b.trims as f64, b.travel) } else { (a.travel, a.trims as f64, b.travel, b.trims as f64) };
    a1 < b1 || (a1 == b1 && a2 < b2)
}

/// The cheapest ways for `items` in their order, exactly, by a dynamic programme: trims first
/// then travel, or travel first then trims. `None` when an item has no way.
fn cheapest(items: &[Item], ends: &Ends, trims_first: bool) -> Option<Vec<usize>> {
    let first = items.first()?;
    if items.iter().any(|it| it.ways.is_empty()) {
        return None;
    }
    // best[w]: the cheapest cost of sewing the items so far with the last one sewn by way w.
    let mut best: Vec<Cost> = first.ways.iter().map(|w| join(None, first, w, ends)).collect();
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(items.len());
    back.push(vec![0; first.ways.len()]);
    for pair in items.windows(2) {
        let (Some(prev), Some(item)) = (pair.first(), pair.get(1)) else { continue };
        let mut next = Vec::with_capacity(item.ways.len());
        let mut from = Vec::with_capacity(item.ways.len());
        for w in &item.ways {
            let mut pick: Option<(Cost, usize)> = None;
            for (pw, (pc, pway)) in best.iter().zip(&prev.ways).enumerate() {
                let c = add(*pc, join(Some((pway.exit, prev.last_rgb)), item, w, ends));
                if pick.as_ref().is_none_or(|(b, _)| better(&c, b, trims_first)) {
                    pick = Some((c, pw));
                }
            }
            let (c, pw) = pick?;
            next.push(c);
            from.push(pw);
        }
        best = next;
        back.push(from);
    }
    let last = items.last()?;
    let mut pick: Option<(Cost, usize)> = None;
    for (w, (c, way)) in best.iter().zip(&last.ways).enumerate() {
        let mut c = *c;
        if let Some(end) = ends.end {
            c.travel += way.exit.dist(end);
        }
        if pick.as_ref().is_none_or(|(b, _)| better(&c, b, trims_first)) {
            pick = Some((c, w));
        }
    }
    let (_, mut w) = pick?;
    let mut out = vec![0; items.len()];
    for (i, from) in back.iter().enumerate().rev() {
        if let Some(slot) = out.get_mut(i) {
            *slot = w;
        }
        w = from.get(w).copied().unwrap_or(0);
    }
    Some(out)
}

/// Closest join: for `items` sewn in their order, the way each is sewn so that each is entered
/// nearest where the one before it was left (the first nearest the design start, the last left
/// nearest the design end). It never costs more trims or more travel than `current`, the ways
/// they are sewn now: the exact cheapest ways (trims first, else travel first) when one of them
/// is no worse on both and better on one, else `current` unchanged. A pinned item (one way) keeps
/// it.
pub fn closest_join(items: &[Item], current: &[usize], ends: &Ends) -> Vec<usize> {
    let now: Vec<usize> = (0..items.len()).map(|i| current.get(i).copied().unwrap_or(0)).collect();
    let base = cost(items, &now, ends);
    for trims_first in [true, false] {
        let Some(ways) = cheapest(items, ends, trims_first) else { continue };
        let c = cost(items, &ways, ends);
        // Float sums in another order may differ in the last bits.
        let no_worse = c.trims <= base.trims && c.travel <= base.travel + 1e-6;
        // Only for a real gain: an order already as cheap keeps its ways, so joining twice
        // changes nothing the second time.
        let better = c.trims < base.trims || c.travel < base.travel - 1e-6;
        if no_worse && better {
            return ways;
        }
    }
    now
}

/// Sort by colour: `items` grouped by the thread they start in, the groups in the order their
/// colour first appears, the items of each group in their order. A permutation of indices into
/// `items`; the order unchanged when grouping would not lower the thread changes (it never
/// raises them).
pub fn colour_sort(items: &[Item]) -> Vec<usize> {
    let identity: Vec<usize> = (0..items.len()).collect();
    let mut colours: Vec<[u8; 3]> = vec![];
    for it in items {
        if !colours.contains(&it.first_rgb) {
            colours.push(it.first_rgb);
        }
    }
    let sorted: Vec<usize> =
        colours.iter().flat_map(|c| items.iter().enumerate().filter(move |(_, it)| it.first_rgb == *c).map(|(i, _)| i)).collect();
    if colour_changes(items, &sorted) < colour_changes(items, &identity) { sorted } else { identity }
}

/// The sewing order of `natural` (the objects in document order) when `listed` is sewn first:
/// indices into `natural`, each exactly once: those listed, in their listed order (ids not in
/// `natural` skipped, repeats counted once), then the rest in document order.
pub fn resolve_order<T: Copy + Eq + Hash>(listed: &[T], natural: &[T]) -> Vec<usize> {
    let mut at: std::collections::HashMap<T, usize> = std::collections::HashMap::with_capacity(natural.len());
    for (i, id) in natural.iter().enumerate() {
        at.entry(*id).or_insert(i);
    }
    let mut taken: HashSet<usize> = HashSet::with_capacity(natural.len());
    let mut out = Vec::with_capacity(natural.len());
    for id in listed {
        if let Some(&i) = at.get(id)
            && taken.insert(i)
        {
            out.push(i);
        }
    }
    out.extend((0..natural.len()).filter(|i| !taken.contains(i)));
    out
}

/// Which of `ways` is entered nearest `entry` and left nearest `exit` (either may be unset, in
/// which case it does not count); `None` when both are unset or there are no ways.
pub fn nearest_way(ways: &[Way], entry: Option<Pt>, exit: Option<Pt>) -> Option<usize> {
    if entry.is_none() && exit.is_none() {
        return None;
    }
    let off = |w: &Way| entry.map_or(0.0, |p| p.dist(w.entry)) + exit.map_or(0.0, |p| p.dist(w.exit));
    let mut best: Option<(f64, usize)> = None;
    for (i, w) in ways.iter().enumerate() {
        let d = off(w);
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// The ways `items` are sewn: `chosen[i]` where one was chosen for the item (its entry or exit
/// point), else the first way, except that with a design start the first item is entered nearest
/// it, and with a design end the last item is left nearest it.
pub fn current_ways(items: &[Item], chosen: &[Option<usize>], ends: &Ends) -> Vec<usize> {
    let n = items.len();
    let mut out: Vec<usize> = (0..n).map(|i| chosen.get(i).copied().flatten().unwrap_or(0)).collect();
    let free = |i: usize| chosen.get(i).copied().flatten().is_none();
    if let Some(first) = items.first()
        && free(0)
    {
        let end = if n == 1 { ends.end } else { None };
        if let Some(w) = nearest_way(&first.ways, ends.start, end)
            && let Some(slot) = out.first_mut()
        {
            *slot = w;
        }
    }
    if n > 1
        && free(n - 1)
        && let Some(last) = items.last()
        && let Some(w) = nearest_way(&last.ways, None, ends.end)
        && let Some(slot) = out.last_mut()
    {
        *slot = w;
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn pt(x: i32, y: i32) -> Pt {
        Pt { x, y }
    }

    /// A line from `a` to `b` that can be sewn either way.
    fn line(rgb: [u8; 3], a: Pt, b: Pt) -> Item {
        Item {
            first_rgb: rgb,
            last_rgb: rgb,
            changes: 0,
            ways: vec![Way { entry: a, exit: b, trims: 0, travel: 0.0 }, Way { entry: b, exit: a, trims: 0, travel: 0.0 }],
        }
    }

    #[test]
    fn closest_join_reverses_lines_to_meet_end_to_end() {
        // Three lines laid end to end, the middle one drawn backwards.
        let items = vec![line(RED, pt(0, 0), pt(100, 0)), line(RED, pt(200, 0), pt(100, 0)), line(RED, pt(200, 0), pt(300, 0))];
        let ends = Ends { min_stitch: 3, ..Default::default() };
        let before = cost(&items, &[0, 0, 0], &ends);
        assert_eq!((before.trims, before.travel), (2, 200.0));
        let ways = closest_join(&items, &[0, 0, 0], &ends);
        assert_eq!(ways, vec![0, 1, 0]);
        let after = cost(&items, &ways, &ends);
        assert_eq!((after.trims, after.travel), (0, 0.0));
    }

    #[test]
    fn a_pinned_item_keeps_its_way_and_the_ends_count() {
        let mut pinned = line(RED, pt(0, 0), pt(100, 0));
        pinned.ways.truncate(1);
        let items = vec![pinned, line(RED, pt(500, 0), pt(100, 0))];
        let ends = Ends { start: Some(pt(100, 0)), end: Some(pt(500, 0)), min_stitch: 3 };
        let ways = closest_join(&items, &[0, 0], &ends);
        assert_eq!(ways, vec![0, 1]);
        // Without a choice the start and end still pick the first and last ways.
        let one = [line(RED, pt(0, 0), pt(10, 0))];
        assert_eq!(current_ways(&one, &[None], &Ends { start: Some(pt(10, 0)), end: None, min_stitch: 3 }), vec![1]);
        assert_eq!(current_ways(&one, &[Some(0)], &Ends { start: Some(pt(10, 0)), end: None, min_stitch: 3 }), vec![0], "a chosen way stays");
    }

    #[test]
    fn colour_sort_groups_by_first_appearance_keeping_order() {
        let a = |rgb| line(rgb, pt(0, 0), pt(1, 0));
        let items = vec![a(RED), a(BLUE), a(RED), a(BLUE)];
        assert_eq!(colour_changes(&items, &[0, 1, 2, 3]), 3);
        let order = colour_sort(&items);
        assert_eq!(order, vec![0, 2, 1, 3]);
        assert_eq!(colour_changes(&items, &order), 1);
    }

    #[test]
    fn resolve_order_lists_each_object_once() {
        assert_eq!(resolve_order(&[30, 99, 10, 30], &[10, 20, 30]), vec![2, 0, 1]);
        assert_eq!(resolve_order::<u8>(&[], &[1, 2]), vec![0, 1]);
        assert_eq!(nearest_way(&[], Some(pt(0, 0)), None), None);
        assert!(closest_join(&[], &[], &Ends::default()).is_empty());
    }

    fn arb_pt() -> impl Strategy<Value = Pt> {
        (-500i32..500, -500i32..500).prop_map(|(x, y)| Pt { x, y })
    }

    fn arb_item() -> impl Strategy<Value = Item> {
        let colours = prop::sample::select(vec![RED, BLUE, [0, 160, 0]]);
        (colours.clone(), colours, prop::collection::vec((arb_pt(), arb_pt(), 0usize..3, 0.0f64..50.0), 1..6)).prop_map(|(a, b, ways)| Item {
            first_rgb: a,
            last_rgb: b,
            changes: usize::from(a != b),
            ways: ways.into_iter().map(|(entry, exit, trims, travel)| Way { entry, exit, trims, travel }).collect(),
        })
    }

    proptest! {
        #[test]
        fn closest_join_never_adds_trims_or_travel(
            items in prop::collection::vec(arb_item(), 0..14),
            pick in prop::collection::vec(0usize..6, 14),
            start in prop::option::of(arb_pt()),
            end in prop::option::of(arb_pt()),
            min_stitch in 0i32..20,
        ) {
            let current: Vec<usize> = items.iter().zip(&pick).map(|(it, p)| p % it.ways.len()).collect();
            let ends = Ends { start, end, min_stitch };
            let before = cost(&items, &current, &ends);
            let ways = closest_join(&items, &current, &ends);
            prop_assert_eq!(ways.len(), items.len());
            for (w, it) in ways.iter().zip(&items) {
                prop_assert!(*w < it.ways.len());
            }
            let after = cost(&items, &ways, &ends);
            prop_assert!(after.trims <= before.trims, "{before:?} → {after:?}");
            prop_assert!(after.travel <= before.travel + 1e-6, "{before:?} → {after:?}");
        }

        #[test]
        fn colour_sort_is_a_permutation_that_never_adds_changes(items in prop::collection::vec(arb_item(), 0..20)) {
            let order = colour_sort(&items);
            let mut seen = order.clone();
            seen.sort_unstable();
            prop_assert_eq!(seen, (0..items.len()).collect::<Vec<_>>());
            let identity: Vec<usize> = (0..items.len()).collect();
            prop_assert!(colour_changes(&items, &order) <= colour_changes(&items, &identity));
        }

        #[test]
        fn single_colour_items_sort_to_the_fewest_changes(colours in prop::collection::vec(0usize..3, 1..30)) {
            let palette = [RED, BLUE, [0, 160, 0]];
            let items: Vec<Item> = colours.iter().map(|&c| line(palette[c], Pt::default(), Pt::default())).collect();
            let distinct = colours.iter().collect::<HashSet<_>>().len();
            prop_assert_eq!(colour_changes(&items, &colour_sort(&items)), distinct - 1);
        }

        #[test]
        fn any_listed_order_resolves_to_a_permutation(listed in prop::collection::vec(0u32..40, 0..40), n in 0u32..30) {
            let natural: Vec<u32> = (0..n).collect();
            let order = resolve_order(&listed, &natural);
            let mut seen = order.clone();
            seen.sort_unstable();
            prop_assert_eq!(seen, (0..natural.len()).collect::<Vec<_>>());
        }
    }
}
