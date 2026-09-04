//! Manual capture check. CI has no live desktop or Warframe window, so this is `#[ignore]`d and run
//! by hand on a real session:
//!
//!     cargo test -p tennoscope --test capture_harness -- --ignored --nocapture
//!     TENNOSCOPE_REQUIRE_CAPTURE=1 cargo test -p tennoscope --test capture_harness -- --ignored --nocapture
//!
//! The default mode permits a named error when Warframe is closed. The strict environment-variable
//! mode requires at least one captured candidate. Capture is automatic and must not open a desktop
//! picker.

fn validate_capture_outcome(
    outcome: Result<usize, &'static str>,
    require_capture: bool,
) -> Result<(), &'static str> {
    match outcome {
        Ok(0) => Err("capture returned no candidates"),
        Ok(_) => Ok(()),
        Err(reason) if require_capture => Err(reason),
        Err(_) => Ok(()),
    }
}

#[test]
fn default_mode_accepts_a_named_game_closed_error_but_never_an_empty_success() {
    assert_eq!(
        validate_capture_outcome(Err("no Warframe window found"), false),
        Ok(())
    );
    assert_eq!(
        validate_capture_outcome(Ok(0), false),
        Err("capture returned no candidates")
    );
    assert_eq!(validate_capture_outcome(Ok(1), false), Ok(()));
}

#[test]
fn required_capture_mode_rejects_errors_and_accepts_non_empty_success() {
    assert_eq!(
        validate_capture_outcome(Err("no Warframe window found"), true),
        Err("no Warframe window found")
    );
    assert_eq!(
        validate_capture_outcome(Ok(0), true),
        Err("capture returned no candidates")
    );
    assert_eq!(validate_capture_outcome(Ok(2), true), Ok(()));
}

#[test]
#[ignore = "needs a live desktop session and a running Warframe window"]
fn captures_a_frame_from_this_session() {
    let mut capture = app_lib::reward_capture::GameCapture::new();
    let require_capture =
        std::env::var_os("TENNOSCOPE_REQUIRE_CAPTURE").is_some_and(|value| value == "1");
    let outcome = capture.capture_candidates();
    match &outcome {
        Ok(candidates) => {
            for candidate in candidates {
                println!(
                    "captured {}x{} for rect {:?}",
                    candidate.image.width(),
                    candidate.image.height(),
                    candidate.rect
                );
                assert!(
                    candidate.image.width() > 0 && candidate.image.height() > 0,
                    "empty frame"
                );
            }
        }
        Err(reason) => {
            println!("no capture: {reason}");
        }
    }
    validate_capture_outcome(outcome.map(|candidates| candidates.len()), require_capture)
        .unwrap_or_else(|reason| panic!("capture harness failed: {reason}"));
}
