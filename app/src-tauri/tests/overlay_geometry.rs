use app_lib::reward_overlay_geometry;

#[test]
fn reward_overlay_tracks_the_game_reward_row_across_common_displays() {
    let hd = reward_overlay_geometry(1920, 1080, 0, 0);
    assert_eq!((hd.width, hd.height), (1440, 148));
    assert_eq!((hd.x, hd.y), (240, 286));

    let ultrawide = reward_overlay_geometry(3440, 1440, 1920, 0);
    assert_eq!(ultrawide.width, 1600);
    assert_eq!(ultrawide.x, 2840);
    assert_eq!(ultrawide.y, 382);
}
