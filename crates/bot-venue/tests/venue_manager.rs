use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use bot_venue::{SwitchConfig, SwitchError, SwitchPolicy, SwitchPrecheckContext, VenueManager};

fn ready_ctx(now_ms: i64, utc_day: u32, old_open_positions: usize) -> SwitchPrecheckContext {
    SwitchPrecheckContext {
        engine_mode: EngineMode::Ready,
        trading_mode: TradingMode::Paper,
        syncing: false,
        old_open_positions,
        now_ms,
        utc_day,
    }
}

#[test]
fn switch_spot_to_futures_keep_managing_and_drain_to_disabled() {
    let mut mgr = VenueManager::new(Venue::Spot, SwitchConfig::default());
    mgr.set_venue_enabled(Venue::UsdsFutures, true);
    mgr.set_credentials_valid(Venue::UsdsFutures, true);

    let ctx = ready_ctx(1_000_000, 1, 2);
    let old = mgr
        .begin_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &ctx)
        .expect("begin switch");
    assert_eq!(old, Venue::Spot);
    assert_eq!(mgr.state(Venue::UsdsFutures), VenueState::Activating);
    assert_eq!(mgr.state(Venue::Spot), VenueState::Draining);

    mgr.complete_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, 1_001_000, 1);
    assert_eq!(mgr.active_venue(), Venue::UsdsFutures);
    assert_eq!(mgr.state(Venue::UsdsFutures), VenueState::Active);
    assert_eq!(mgr.state(Venue::Spot), VenueState::Draining);

    // Once open positions on draining Spot reach 0, it transitions to Disabled.
    assert!(mgr.mark_draining_flat(Venue::Spot));
    assert_eq!(mgr.state(Venue::Spot), VenueState::Disabled);
    // Calling again is a no-op.
    assert!(!mgr.mark_draining_flat(Venue::Spot));
}

#[test]
fn switch_require_flat_rejected_when_positions_open() {
    let mut mgr = VenueManager::new(Venue::Spot, SwitchConfig::default());
    mgr.set_venue_enabled(Venue::UsdsFutures, true);
    mgr.set_credentials_valid(Venue::UsdsFutures, true);

    let ctx = ready_ctx(1_000_000, 1, 1);
    let err = mgr
        .precheck_switch(Venue::UsdsFutures, SwitchPolicy::RequireFlat, &ctx)
        .expect_err("must reject when open positions > 0");
    assert_eq!(err, SwitchError::OpenPositionsRequireFlat { count: 1 });
}

#[test]
fn switch_rejected_on_halted_killed_syncing_or_already_active() {
    let mut mgr = VenueManager::new(Venue::Spot, SwitchConfig::default());
    mgr.set_venue_enabled(Venue::UsdsFutures, true);
    mgr.set_credentials_valid(Venue::UsdsFutures, true);

    let mut ctx = ready_ctx(1_000_000, 1, 0);
    ctx.engine_mode = EngineMode::Halted;
    assert_eq!(
        mgr.precheck_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &ctx),
        Err(SwitchError::InvalidEngineMode(EngineMode::Halted))
    );

    ctx.engine_mode = EngineMode::Killed;
    assert_eq!(
        mgr.precheck_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &ctx),
        Err(SwitchError::InvalidEngineMode(EngineMode::Killed))
    );

    ctx.engine_mode = EngineMode::Ready;
    ctx.syncing = true;
    assert_eq!(
        mgr.precheck_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &ctx),
        Err(SwitchError::SyncingInProgress)
    );

    ctx.syncing = false;
    assert_eq!(
        mgr.precheck_switch(Venue::Spot, SwitchPolicy::KeepManaging, &ctx),
        Err(SwitchError::AlreadyActive(Venue::Spot))
    );
}

#[test]
fn switch_enforces_live_confirmation_cooldown_and_daily_limit() {
    let mut mgr = VenueManager::new(Venue::Spot, SwitchConfig::default());
    mgr.set_venue_enabled(Venue::UsdsFutures, true);
    mgr.set_credentials_valid(Venue::UsdsFutures, true);

    let mut live_ctx = ready_ctx(1_000_000, 1, 0);
    live_ctx.trading_mode = TradingMode::Live;
    assert_eq!(
        mgr.precheck_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &live_ctx),
        Err(SwitchError::MissingLiveConfirmation(Venue::UsdsFutures))
    );

    // Perform 3 switches with >= 60s spacing, then 4th on same UTC day must be rejected.
    let mut now = 1_000_000_i64;
    let targets = [Venue::UsdsFutures, Venue::Spot, Venue::UsdsFutures];
    for target in targets {
        let ctx = ready_ctx(now, 1, 0);
        mgr.begin_switch(target, SwitchPolicy::Flatten, &ctx)
            .expect("switch ok");
        mgr.complete_switch(target, SwitchPolicy::Flatten, now, 1);

        // Immediately trying again within 60s fails cooldown.
        let soon_ctx = ready_ctx(now + 10_000, 1, 0);
        let next_target = if target == Venue::Spot {
            Venue::UsdsFutures
        } else {
            Venue::Spot
        };
        if mgr.switches_today(1) < 3 {
            assert!(matches!(
                mgr.precheck_switch(next_target, SwitchPolicy::Flatten, &soon_ctx),
                Err(SwitchError::CooldownActive { .. })
            ));
        }
        now += 61_000;
    }

    let fourth_ctx = ready_ctx(now, 1, 0);
    assert_eq!(
        mgr.precheck_switch(Venue::Spot, SwitchPolicy::Flatten, &fourth_ctx),
        Err(SwitchError::DailyLimitReached { limit: 3 })
    );

    // Next UTC day resets the daily switch counter.
    let next_day_ctx = ready_ctx(now + 86_400_000, 2, 0);
    assert!(mgr
        .precheck_switch(Venue::Spot, SwitchPolicy::Flatten, &next_day_ctx)
        .is_ok());
}

#[test]
fn rollback_switch_restores_old_venue_and_disables_target() {
    let mut mgr = VenueManager::new(Venue::Spot, SwitchConfig::default());
    mgr.set_venue_enabled(Venue::UsdsFutures, true);
    mgr.set_credentials_valid(Venue::UsdsFutures, true);

    let ctx = ready_ctx(1_000_000, 1, 1);
    let old = mgr
        .begin_switch(Venue::UsdsFutures, SwitchPolicy::KeepManaging, &ctx)
        .expect("begin switch");
    assert_eq!(old, Venue::Spot);
    assert_eq!(mgr.state(Venue::UsdsFutures), VenueState::Activating);

    mgr.rollback_switch(old, Venue::UsdsFutures);
    assert_eq!(mgr.active_venue(), Venue::Spot);
    assert_eq!(mgr.state(Venue::Spot), VenueState::Active);
    assert_eq!(mgr.state(Venue::UsdsFutures), VenueState::Disabled);
}
