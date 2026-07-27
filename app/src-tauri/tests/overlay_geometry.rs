use app_lib::{reward_overlay_geometry, warframe_window_rect_from_sway_tree};

/// The overlay has to line up with the game's four reward cards, which on a 1920x1080 screen span
/// x=478 to x=1444 and bottom out at y=525 under the player-name row. It used to be sized at 75% of
/// the screen and placed at 56% of the height, which made it 1440px wide against the cards' 966 and
/// put it roughly 75px below them.
#[test]
fn reward_overlay_sits_under_the_game_reward_cards() {
    let hd = reward_overlay_geometry(1920, 1080, 0, 0);
    assert_eq!((hd.x, hd.width), (478, 966), "must span the card block");
    assert_eq!(
        (hd.y, hd.height),
        (530, 156),
        "must sit just below the cards"
    );

    // Same screen, second monitor: the block is offset but keeps its size.
    let offset = reward_overlay_geometry(1920, 1080, 1920, 0);
    assert_eq!((offset.x, offset.y), (2398, 530));
    assert_eq!((offset.width, offset.height), (966, 156));

    // Scaling is proportional, with no clamp to break the alignment.
    let ultrawide = reward_overlay_geometry(3440, 1440, 1920, 0);
    assert_eq!((ultrawide.x, ultrawide.width), (2776, 1731));
    assert_eq!((ultrawide.y, ultrawide.height), (707, 208));
}

#[test]
fn sway_tree_locator_targets_the_visible_warframe_window() {
    let tree = br#"{
      "nodes":[
        {"name":"TennoScope","app_id":"TennoScope","visible":true,"rect":{"x":0,"y":0,"width":960,"height":1080},"nodes":[],"floating_nodes":[]},
        {"name":"Warframe","app_id":null,"visible":true,"window_properties":{"class":"steam_app_warframe"},"rect":{"x":1920,"y":0,"width":1920,"height":1080},"nodes":[],"floating_nodes":[]}
      ],
      "floating_nodes":[]
    }"#;
    let rect = warframe_window_rect_from_sway_tree(tree).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (1920, 0, 1920, 1080)
    );
    let overlay = reward_overlay_geometry(rect.width, rect.height, rect.x, rect.y);
    assert_eq!((overlay.x, overlay.y), (2398, 530));
}
