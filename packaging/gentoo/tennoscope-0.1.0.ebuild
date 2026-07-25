EAPI=8

inherit desktop xdg

DESCRIPTION="Local-first Linux Warframe collection and relic companion"
S="${WORKDIR}/${P}"

LICENSE="GPL-3"
SLOT="0"
KEYWORDS="~amd64"

RDEPEND="
	app-text/tesseract
	dev-libs/glib:2
	gui-apps/grim
	gui-libs/gtk-layer-shell
	net-libs/libsoup:3.0
	net-libs/webkit-gtk:4.1
	x11-libs/gtk+:3
"
DEPEND="${RDEPEND}"
BDEPEND="
	>=net-libs/nodejs-20.19[corepack(+)]
	virtual/pkgconfig
	|| (
		>=dev-lang/rust-1.85.0
		>=dev-lang/rust-bin-1.85.0
	)
"

# This local-overlay ebuild resolves the lockfile's npm and Cargo sources during
# prepare/build. A repository submission must replace this with fully enumerated
# offline source artifacts.

src_unpack() {
	[[ -r ${DISTDIR}/${P}.tar.gz ]] || die "place ${P}.tar.gz in DISTDIR"
	unpack "${P}.tar.gz"
}

src_prepare() {
	default
	export COREPACK_HOME="${T}/corepack"
	export PNPM_HOME="${T}/pnpm"
	corepack pnpm --dir app install --frozen-lockfile || die
}

src_compile() {
	export CARGO_HOME="${T}/cargo"
	export CARGO_TARGET_DIR="${WORKDIR}/target"
	corepack pnpm --dir app build || die
	cargo build --release --locked -p warframe-helper --features tauri/custom-protocol || die
}

src_test() {
	export CARGO_HOME="${T}/cargo"
	export CARGO_TARGET_DIR="${WORKDIR}/target"
	cargo test --workspace --locked || die
	corepack pnpm --dir app check || die
}

src_install() {
	dobin "${WORKDIR}/target/release/tennoscope"
	domenu packaging/tennoscope.desktop
	newicon -s 128 app/src-tauri/icons/128x128.png tennoscope.png
	dodoc LICENSE THIRD_PARTY_NOTICES.md
}
