#![cfg(feature = "tz")]

use std::time::Duration;

use pacer::sim::{Step, simulate};
use pacer::tz::{LocalTime, zone};
use pacer::{Pacer, Time, min_gap, not_after, zoned_window};

/// A reminder at most once a day, only 08:00 to 18:00 Berlin, until a
/// deadline: over the switch back to winter time (25 October 2026).
#[test]
fn a_daily_reminder_in_office_hours_over_the_dst_switch() {
    struct Sent {
        at: Vec<Time>,
        deadline: Time,
    }
    let berlin = zone("Europe/Berlin");
    let pacer = Pacer::new(vec![
        Box::new(zoned_window(
            berlin.clone(),
            LocalTime::constant(8, 0, 0, 0),
            LocalTime::constant(18, 0, 0, 0),
        )) as Box<dyn pacer::Rule<Sent>>,
        Box::new(min_gap(
            |_: &Sent| "r".into(),
            Duration::from_secs(86_400),
            |s: &Sent, _: &str| s.at.last().copied(),
        )),
        Box::new(not_after(|s: &Sent| Some(s.deadline))),
    ]);
    let from: Time = "2026-10-23T20:00:00Z".parse().unwrap();
    let mut sent = Sent {
        at: vec![],
        deadline: "2026-10-26T12:00:00Z".parse().unwrap(),
    };
    let run = simulate(
        &pacer,
        &mut sent,
        from,
        from + Duration::from_secs(5 * 86_400),
        |s, t| s.at.push(t),
    );
    // 24 h after 08:00 CEST is 07:00 CET, outside the window: the next one
    // waits for 08:00 CET. The check on the 27th is past the deadline.
    assert_eq!(
        run.in_zone(&berlin),
        "\
2026-10-24T08:00:00 acted
2026-10-25T08:00:00 acted
2026-10-26T08:00:00 acted
2026-10-27T08:00:00 stopped by not-after
"
    );
    assert!(matches!(run.steps.last(), Some(Step::Stopped { .. })));
}

#[test]
#[should_panic(expected = "does the action change the context?")]
fn an_action_that_changes_nothing_is_caught() {
    let pacer: Pacer<()> = Pacer::new(vec![]);
    simulate(
        &pacer,
        &mut (),
        Time::UNIX_EPOCH,
        Time::from_secs(1),
        |_, _| {},
    );
}
