import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getKioskView, type CellChip, type KioskMastery, type KioskView } from './backend'
import { MetalMark } from './MetalMark'

/*
 * Screen geometry, mirrored from `src-tauri/src/kiosk_geometry.rs`. Warframe scales its HUD
 * with window height and centres it horizontally, so every position here is a design pixel at
 * the 1920x1080 calibration multiplied by `--h` (one design pixel's actual size); horizontal
 * positions are offsets from the window's horizontal centre. Grid chips anchor at their tile
 * corner by their RIGHT edge, basket and total pairs by their bottom-right corner (the box
 * descent puts the digits' baseline on the game's), matching how the game lays out its own.
 */
const CAL = 1080
const fx = (px: number) => px / CAL
const fcx = (px: number) => (px - 960) / CAL

/*
 * Each card border stroke, measured per column off a live capture. The 207.5px pitch
 * rasterizes borders on alternating half-pixels, so a pitch formula rounded once per chip
 * drifted up to 5px by the last column; the measured positions are the only pixel-exact
 * source. Chips sit flush with their card's top-right corner.
 */
const COL_RIGHTS = [264, 471.5, 679.5, 887, 1094.5, 1302.5].map(fcx)
const ROW_TOPS = [199, 421, 643, 865].map(fx)

/// Digits' baseline sits ~3 design px above a line-height-1 box's bottom edge.
const PAIR_DESCENT = fx(3)

const ROW_PAIR_RIGHT = fcx(1750)
const BASKET_FIRST_BASELINE = fx(243)
const BASKET_PITCH = fx(115 / 3)

const TOTAL_PAIR_RIGHT = fcx(1700)
const TOTAL_BASELINE = fx(875)

/** Design pixels -> CSS calc against `--h`, for a `left` anchored at the window centre. An
 * edge on a border stroke's centre renders as its floor: the stroke's last full pixel. */
const cx = (fraction: number) => `calc(50% + ${Math.floor(fraction * CAL)} * var(--h))`
/** Design pixels -> CSS calc from the top edge. */
const y = (fraction: number) => `calc(${Math.round(fraction * CAL)} * var(--h))`

/* The strip's room, mirrored from `src-tauri/src/kiosk_geometry.rs`: the tile's width less the
 * reach of the game's owned badge, which widens for a two-digit count. */
const TILE_W = 190
const BADGE_CLEARANCE = 51
const BADGE_CLEARANCE_TWO_DIGITS = 63

/** Where the no-live-inventory note sits, in absolute design pixels of the calibration. The
 * rest of the overlay measures from the window centre, because everything else on it is. */
const NOTE_LEFT = 150
const NOTE_TOP = 154

/** Design pixels -> CSS calc from a layer's left edge, for a point given absolutely rather
 * than as an offset from the centre. */
const dx = (px: number) => `calc(${Math.round(px)} * var(--h))`

/** How wide a strip for this part may be before it reaches the game's owned badge. */
function stripMaxWidth(held: number): string {
  const clearance = held >= 10 ? BADGE_CLEARANCE_TWO_DIGITS : BADGE_CLEARANCE
  return `calc(${TILE_W - clearance} * var(--h))`
}

function gridChipStyle(col: number, row: number): React.CSSProperties {
  return {
    left: cx(COL_RIGHTS[col]),
    top: y(ROW_TOPS[row]),
    transform: 'translateX(-100%)',
  }
}

function basketChipStyle(index: number): React.CSSProperties {
  const baseline = BASKET_FIRST_BASELINE + BASKET_PITCH * index
  return {
    left: cx(ROW_PAIR_RIGHT),
    top: y(baseline + PAIR_DESCENT),
    transform: 'translate(-100%, -100%)',
  }
}

const totalChipStyle: React.CSSProperties = {
  left: cx(TOTAL_PAIR_RIGHT),
  top: y(TOTAL_BASELINE + PAIR_DESCENT),
  transform: 'translate(-100%, -100%)',
}

const noteStyle: React.CSSProperties = { left: dx(NOTE_LEFT), top: y(fx(NOTE_TOP)) }

/** An unmastered part in place of its price chip. Installed fonts decide its real width, so the
 * built box is measured and detail drops until it fits; the word and the price never drop. */
function MasteryStrip({ cell, mastery }: { cell: CellChip, mastery: KioskMastery }) {
  const [tier, setTier] = useState<1 | 2 | 3>(1)
  const strip = useRef<HTMLSpanElement>(null)
  const fitted = useRef('')
  const signature = `${cell.name} ${cell.platinum} ${mastery.held} ${mastery.uses}`

  useLayoutEffect(() => {
    // A different reading is a different strip to fit, so a tier chosen for the last one says
    // nothing about this one: start the search over before measuring anything.
    if (fitted.current !== signature) {
      fitted.current = signature
      if (tier !== 1) {
        setTier(1)
        return
      }
    }
    const element = strip.current
    if (!element || element.scrollWidth <= element.clientWidth) return
    // Step down only from the tier this render measured, so a second run for the same render cannot
    // skip one, and stop at the last tier so the loop settles.
    setTier(current => (current === tier && current < 3 ? ((current + 1) as 2 | 3) : current))
  }, [tier, signature])

  return <span
    ref={strip}
    className="kiosk-chip kiosk-strip-chip"
    data-testid="kiosk-mastery-strip"
    data-tier={tier}
    style={{ ...gridChipStyle(cell.col, cell.row), maxWidth: stripMaxWidth(mastery.held) }}
    title={cell.name}
  >
    <span className="kiosk-fact">Unmastered</span>
    {mastery.uses >= 2 && tier < 3 &&
      <span className="kiosk-fraction">{mastery.held}/{mastery.uses}</span>}
    {tier < 2 && <MetalMark metal="plat" className="kiosk-mark"/>}
    <b className="kiosk-price">{cell.platinum === null ? '—' : `${cell.platinum}p`}</b>
  </span>
}

export default function KioskOverlay() {
  const [view, setView] = useState<KioskView | null>(null)
  const [faded, setFaded] = useState(false)
  const [offset, setOffset] = useState(0)
  const epochSeen = useRef(-1)
  const sessionSeen = useRef(-1)
  // How many scroll deltas have landed. A settled read that started before the last one is
  // a measurement of a grid that no longer exists, and its absolute must be dropped.
  const scrollSeq = useRef(0)
  // `kiosk-updated` events can overlap IPC reads. Only the newest-started read may publish;
  // otherwise a slow older response can roll the epoch, prices, and absolute offset back.
  const refreshSeq = useRef(0)
  // Unreadable looks fade the chips; a settled read unhides them again, but only when no
  // unreadable look landed while the read was in flight, otherwise a stale pre-dialog view
  // would briefly paint chips over the dialog until the next null re-fades them.
  const fadeSeq = useRef(0)
  // A single torn frame or one mid-animation look is not blindness; the backend already only
  // emits null for unmeasurable or absent strips, but its 60ms cadence means one noisy look
  // could blink a good view. The fade lands only after this many consecutive nulls, so
  // isolated misses cost nothing and genuine occlusion (a dialog up, a cinematic over the
  // pane) still fades within a few ticks.
  const NULL_STREAK_TO_FADE = 3
  const nullStreak = useRef(0)

  useEffect(() => {
    document.documentElement.classList.add('overlay-mode')
    let active = true
    let unlistenUpdated: UnlistenFn | undefined
    let unlistenScroll: UnlistenFn | undefined

    const adoptSession = (session: number | null) => {
      const nextSession = session ?? -1
      if (nextSession === sessionSeen.current) return
      sessionSeen.current = nextSession
      epochSeen.current = -1
      scrollSeq.current = 0
      // A streak from the previous visit must not fade the new one: the first
      // torn frame after opening would otherwise trip an inherited count.
      nullStreak.current = 0
      setOffset(0)
      setFaded(false)
      setView(null)
    }
    // A published epoch is fetched once; the payload-in-event would race the window still
    // loading, so the event is only a nudge and `get_kiosk_view` is the source of truth.
    const refresh = async () => {
      const refreshId = ++refreshSeq.current
      const fadeAtRead = fadeSeq.current
      try {
        const seqAtRead = scrollSeq.current
        const next = await getKioskView()
        if (!active || refreshId !== refreshSeq.current) return
        if (!next) {
          adoptSession(null)
          return
        }
        const sessionChanged = next.session !== sessionSeen.current
        if (sessionChanged) adoptSession(next.session)
        // A settled read means the grid was readable when the read began, but only when no
        // unreadable look landed while it was in flight. Otherwise the settling view predates
        // the occlusion (an unreadable strip look during a dialog) and unhiding would paint
        // stale chips over it until the next null re-fades them. Gating the old epoch check
        // instead latched a single unreadable look into a permanently invisible overlay,
        // because dialogs come and go mid-epoch while the epoch only advances on a re-anchor.
        nullStreak.current = 0
        if (fadeAtRead === fadeSeq.current) setFaded(false)
        // Every settled read measured where the grid sits right now, so its offset is
        // authoritative whenever nothing has moved since the read began, not just when
        // an anchor marks it. Adopting only anchors lets each look's estimation error
        // compound unrestrained until the chips drift clean off their cards mid-session.
        if (sessionChanged || seqAtRead === scrollSeq.current) {
          setOffset(next.scroll_dy)
        }
        epochSeen.current = next.epoch
        setView(next)
      } catch { /* transient IPC failure: keep the last anchor standing */ }
    }

    // While the grid moves the backend streams how far it moved since the last look. The
    // chips ride the scroll by accumulating those deltas. An unreadable look (null) fades
    // them until the next settled read publishes where the grid actually is, but only
    // after a short run of nulls, so one noisy look cannot blink a good view.
    void listen<{ session: number, dy: number | null }>('kiosk-scroll', (event) => {
      if (!active || event.payload.session !== sessionSeen.current) return
      if (event.payload.dy === null) {
        nullStreak.current += 1
        if (nullStreak.current < NULL_STREAK_TO_FADE) return
        fadeSeq.current += 1
        setFaded(true)
        return
      }
      nullStreak.current = 0
      const delta = event.payload.dy
      scrollSeq.current += 1
      setOffset(previous => previous + delta)
    }).then(stop => { if (active) unlistenScroll = stop; else stop() })

    void listen<number | null>('kiosk-updated', (event) => {
      adoptSession(event.payload)
      void refresh()
    }).then(stop => {
      if (active) unlistenUpdated = stop
      else stop()
    })

    void refresh()
    return () => {
      active = false
      unlistenUpdated?.()
      unlistenScroll?.()
      document.documentElement.classList.remove('overlay-mode')
    }
  }, [])
  // Hoisted out of the map because the callback cannot narrow the view it closes over.
  const masteryLive = view?.mastery_status === 'live'

  return <main className="kiosk-shell" aria-label="Kiosk overlay">
    <div
      className={faded ? 'kiosk-strip kiosk-faded' : 'kiosk-strip'}
      data-testid="kiosk-strip"
    >
      {/* Only the grid scrolls; the basket pane is fixed in the game, so its chips stay put. */}
      <div
        className="kiosk-grid"
        data-testid="kiosk-grid"
        style={{
          transform: `translateY(calc(${offset} * var(--h)))`,
          // The pane's own clip edges (193 and 983 design px, measured on a live frame): a
          // row sliding out of the pane must slide out of the overlay too, instead of
          // painting its chips over the header. The inset is undone by the same translate
          // the chips ride, so it is expressed against the untranslated box.
          clipPath: `inset(calc(${193 - offset} * var(--h)) 0 calc(${97 + offset} * var(--h)) 0)`,
        }}
      >
        {view?.cells.map(cell =>
          // Only this run's live inventory backs a strip; without it the tile stays priced, since
          // a strip would brand every part on screen unmastered.
          masteryLive && cell.mastery
            ? <MasteryStrip key={`${cell.col}:${cell.row}`} cell={cell} mastery={cell.mastery}/>
            : <span
              key={`${cell.col}:${cell.row}`}
              className="kiosk-chip"
              data-testid="kiosk-grid-chip"
              style={gridChipStyle(cell.col, cell.row)}
              title={cell.name}
            >
              <MetalMark metal="plat" className="kiosk-mark"/>{cell.platinum}p
            </span>
        )}
      </div>
      {/* Outside the scrolling grid, because it reports the whole view rather than a row of
          it, and it fades with the chips: an overlay whose numbers have gone unreadable has
          no standing to claim anything is missing. */}
      {view?.mastery_status === 'unavailable' &&
        <span className="kiosk-note" style={noteStyle}>Mastery: no live inventory</span>}
      {view?.basket.map(row =>
        <span
          key={row.index}
          className="kiosk-pair"
          data-testid="kiosk-basket-chip"
          style={basketChipStyle(row.index)}
          title={row.name}
        >
          <MetalMark metal="plat" className="kiosk-mark"/>
          <b>{row.platinum === null ? '—' : `${row.platinum}p`}</b>
        </span>
      )}
      {view && view.basket.length > 0 &&
        <span className="kiosk-pair kiosk-total" data-testid="kiosk-total" style={totalChipStyle}>
          <MetalMark metal="plat" className="kiosk-mark"/><b>{view.total_plat}p</b>
        </span>}
    </div>
  </main>
}
