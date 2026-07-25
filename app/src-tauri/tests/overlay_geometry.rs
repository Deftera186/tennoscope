use app_lib::{reward_overlay_geometry, warframe_window_rect_from_sway_tree};

#[test]
fn reward_overlay_tracks_the_game_reward_row_across_common_displays() {
    let hd = reward_overlay_geometry(1920, 1080, 0, 0);
    assert_eq!((hd.width, hd.height), (1440, 148));
    assert_eq!((hd.x, hd.y), (240, 605));

    let ultrawide = reward_overlay_geometry(3440, 1440, 1920, 0);
    assert_eq!(ultrawide.width, 1600);
    assert_eq!(ultrawide.x, 2840);
    assert_eq!(ultrawide.y, 806);
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
    assert_eq!((overlay.x, overlay.y), (2160, 605));
}
