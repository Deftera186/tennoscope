import { useEffect, useRef, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getKioskView, type KioskView } from './backend'
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

const COL_PITCH = fx(207.5)
const TILE_W = fx(190)
const GRID_LEFT = fcx(76)
const ROW_TOPS = [199, 421, 643].map(fx)
const CHIP_INSET = fx(6)
const CHIP_RISE = fx(2)

/// Digits' baseline sits ~3 design px above a line-height-1 box's bottom edge.
const PAIR_DESCENT = fx(3)

const ROW_PAIR_RIGHT = fcx(1750)
const BASKET_FIRST_BASELINE = fx(243)
const BASKET_PITCH = fx(115 / 3)

const TOTAL_PAIR_RIGHT = fcx(1717)
const TOTAL_BASELINE = fx(875)

/** Design pixels -> CSS calc against `--h`, for a `left` anchored at the window centre. */
const cx = (fraction: number) => `calc(50% + ${Math.round(fraction * CAL)} * var(--h))`
/** Design pixels -> CSS calc from the top edge. */
const y = (fraction: number) => `calc(${Math.round(fraction * CAL)} * var(--h))`

function gridChipStyle(col: number, row: number): React.CSSProperties {
  return {
    left: cx(GRID_LEFT + COL_PITCH * col + TILE_W - CHIP_INSET),
    top: y(ROW_TOPS[row] - CHIP_RISE),
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

export default function KioskOverlay() {
  const [view, setView] = useState<KioskView | null>(null)
  const [faded, setFaded] = useState(false)
  const [offset, setOffset] = useState(0)
  const epochSeen = useRef(-1)

  useEffect(() => {
    document.documentElement.classList.add('overlay-mode')
    let active = true
    let unlistenUpdated: UnlistenFn | undefined
    let unlistenScroll: UnlistenFn | undefined

    // A published epoch is fetched once; the payload-in-event would race the window still
    // loading, so the event is only a nudge and `get_kiosk_view` is the source of truth.
    const refresh = async () => {
      try {
        const next = await getKioskView()
        if (!active) return
        if (!next) { setView(null); return }
        if (next.epoch !== epochSeen.current) {
          setFaded(false)
          setOffset(0)
        }
        epochSeen.current = next.epoch
        setView(next)
      } catch { /* transient IPC failure: keep the last anchor standing */ }
    }

    // While the grid moves the backend streams its offset from the anchor -- the chips ride
    // the scroll in real time; an unreadable look (null) fades until the next anchor.
    void listen<number | null>('kiosk-scroll', (event) => {
      if (!active) return
      if (event.payload === null) { setFaded(true); return }
      setOffset(event.payload)
    }).then(stop => { if (active) unlistenScroll = stop; else stop() })

    void listen('kiosk-updated', () => { void refresh() }).then(stop => {
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

  return <main className="kiosk-shell" aria-label="Kiosk overlay">
    <div
      className={faded ? 'kiosk-strip kiosk-faded' : 'kiosk-strip'}
      data-testid="kiosk-strip"
      style={{ transform: `translateY(calc(${offset} * var(--h)))` }}
    >
      {view?.cells.map(cell =>
        <span
          key={`${cell.col}:${cell.row}`}
          className="kiosk-chip"
          data-testid="kiosk-grid-chip"
          style={gridChipStyle(cell.col, cell.row)}
          title={cell.name}
        >
          <MetalMark metal="plat" className="kiosk-mark"/>{cell.platinum}p
        </span>
      )}
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
