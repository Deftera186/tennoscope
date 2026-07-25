use app_lib::{RewardObservation, RewardObserverState, match_reward_text, normalize_ocr};

#[test]
fn ocr_normalization_removes_noise_without_destroying_names() {
    assert_eq!(normalize_ocr("  Paris   Prime\nSTRING!  "), "paris prime string");
}

#[test]
fn reward_text_resolves_four_catalog_names_in_screen_order() {
    let catalog = [
        "Paris Prime String",
        "Lex Prime Receiver",
        "Forma Blueprint",
        "Braton Prime Stock",
    ];
    let text = "CHOOSE YOUR REWARD\nParis Prime Strlng\nLex Prime Receiver\nForma Blueprint\nBraton Prime Stock\n15 ducats";

    let rewards = match_reward_text(text, &catalog);

    assert_eq!(
        rewards.iter().map(|reward| reward.name.as_str()).collect::<Vec<_>>(),
        catalog
    );
    assert!(rewards[0].confidence >= 0.8);
}

#[test]
fn observer_debounces_show_and_hide_to_prevent_overlay_flicker() {
    let choices = vec![
        RewardObservation::certain("A"),
        RewardObservation::certain("B"),
        RewardObservation::certain("C"),
        RewardObservation::certain("D"),
    ];
    let mut state = RewardObserverState::new(2, 2);

    assert!(!state.observe(choices.clone()).show);
    let shown = state.observe(choices);
    assert!(shown.show);
    assert_eq!(shown.choices.len(), 4);
    assert!(!state.miss().hide);
    assert!(state.miss().hide);
}
