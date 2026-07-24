use std::time::Duration;

use warframe_acquisition::{
    INVENTORY_ENDPOINT, InventoryHttpTransport, MAX_INVENTORY_RESPONSE_BYTES,
};

#[test]
fn production_transport_is_pinned_to_warframe_https_origin_with_bounded_policy() {
    let transport = InventoryHttpTransport::new().unwrap();

    assert_eq!(
        INVENTORY_ENDPOINT,
        "https://mobile.warframe.com/api/inventory.php"
    );
    assert_eq!(transport.connect_timeout(), Duration::from_secs(5));
    assert_eq!(transport.total_timeout(), Duration::from_secs(20));
    assert_eq!(transport.response_limit(), MAX_INVENTORY_RESPONSE_BYTES);
    assert!(!transport.follows_redirects());
}
