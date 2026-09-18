import { useRef, type KeyboardEvent } from 'react'
import type { AccessMode } from './backend'
import { ACCESS_MODES, ALWAYS_OFF, accessGrantedBetween, accessLevel, accessMode } from './access'

interface AccessSelectorProps {
  value: AccessMode
  onChange: (mode: AccessMode) => void
  effectiveMode?: AccessMode | null
  onConfirmUpgrade?: () => void
  applyingMode?: AccessMode | null
  disabled?: boolean
}

export function AccessSelector({ value, onChange, effectiveMode = null, onConfirmUpgrade, applyingMode = null, disabled = false }: AccessSelectorProps) {
  const radios = useRef<Array<HTMLInputElement | null>>([])
  const selectedIndex = accessLevel(value)
  const selected = accessMode(value)
  const effective = effectiveMode ? accessMode(effectiveMode) : null
  const reviewingUpgrade = effectiveMode !== null && selectedIndex > accessLevel(effectiveMode)
  const reviewingDowngrade = effectiveMode !== null && selectedIndex < accessLevel(effectiveMode)
  const isFirstRun = effectiveMode === null
  const pending = effectiveMode !== null && value !== effectiveMode
  const newlyGranted = reviewingUpgrade && effectiveMode ? accessGrantedBetween(effectiveMode, value) : []
  const retiring = reviewingDowngrade && effectiveMode ? accessGrantedBetween(value, effectiveMode) : []
  const choose = (index: number) => {
    const next = ACCESS_MODES[index]
    if (!next || disabled) return
    onChange(next.id)
    queueMicrotask(() => radios.current[index]?.focus())
  }
  const keyDown = (event: KeyboardEvent<HTMLInputElement>, index: number) => {
    let next = index
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (index - 1 + ACCESS_MODES.length) % ACCESS_MODES.length
    else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (index + 1) % ACCESS_MODES.length
    else if (event.key === 'Home') next = 0
    else if (event.key === 'End') next = ACCESS_MODES.length - 1
    else return
    event.preventDefault()
    choose(next)
  }

  return <div className="access-selector">
    {effective && <div className="access-mode-status" aria-live="polite">
      <span><i aria-hidden="true"/>Effective mode: {effective.name}</span>
      {applyingMode
        ? <span className="access-pending" role="status" aria-label={`Applying ${accessMode(applyingMode).name}`}>Applying {accessMode(applyingMode).name}…</span>
        : pending && <span className="access-pending">Pending: {selected.name}</span>}
    </div>}
    <fieldset className="access-rail" disabled={disabled} aria-label="What may TennoScope observe?">
      <div className="access-stepper">
      <div className="access-track" aria-hidden="true">
        <span className="access-track-line"/>
        <span className="access-track-progress" style={{ transform: `scaleX(${selectedIndex / 2})` }}/>
        {ACCESS_MODES.map((mode, index) => <span key={mode.id} onClick={() => choose(index)} className={`access-node${index <= selectedIndex ? ' included' : ''}${mode.id === effectiveMode ? ' active' : ''}`}/>) }
        <span className="access-thumb" style={{ transform: `translateX(${selectedIndex * 100}%)` }}><span/></span>
      </div>
        <div className="access-stops">
          {ACCESS_MODES.map((mode, index) => <label key={mode.id} className={`access-stop${mode.id === value ? ' selected' : ''}${mode.id === effectiveMode ? ' active' : ''}`}>
            <input ref={node => { radios.current[index] = node }} type="radio" name="access-mode" value={mode.id} checked={mode.id === value} onChange={() => choose(index)} onKeyDown={event => keyDown(event, index)}/>
            <strong>{mode.name}</strong>
            <small>{mode.boundary}</small>
          </label>)}
        </div>
      </div>
    </fieldset>

    <article className={`access-detail${pending && !isFirstRun ? ' review' : ''}`} role="region" aria-label={reviewingUpgrade ? 'New observation to grant' : reviewingDowngrade ? 'Observation to retire' : `What ${selected.name} adds`} aria-live="polite">
      {pending && !isFirstRun ? <header className="access-detail-head">
        <p className="access-eyebrow">Proposed — not yet effective</p>
        <h3>{selected.name}</h3>
      </header> : <div className="access-detail-body">
        <section className="access-observation">
          <h3>What {selected.name} adds</h3>
          <ul>{selected.added.map(item => <li key={item}>{item}</li>)}</ul>
        </section>
      </div>}
      {reviewingUpgrade && <div className="access-detail-body">
        <section className="access-observation">
          <h3>New observation to grant</h3>
          <ul>{newlyGranted.map(item => <li key={item}>{item}</li>)}</ul>
        </section>
        {onConfirmUpgrade && <button type="button" className="stamp" onClick={onConfirmUpgrade} disabled={disabled} aria-busy={disabled}>{disabled ? `Applying ${selected.name}…` : `Confirm ${selected.name}`}</button>}
      </div>}
      {reviewingDowngrade && <div className="access-detail-body">
        <section className="access-observation">
          <h3>Observation to retire</h3>
          <ul>{retiring.map(item => <li key={item}>{item}</li>)}</ul>
        </section>
        {onConfirmUpgrade && <button type="button" className="stamp" onClick={onConfirmUpgrade} disabled={disabled} aria-busy={disabled}>{disabled ? `Applying ${selected.name}…` : `Confirm ${selected.name}`}</button>}
      </div>}
    </article>

    <section className="access-always-off" aria-label="Always prohibited">
      <h3>Always prohibited — at every level</h3>
      <ul>{ALWAYS_OFF.map(item => <li key={item}>{item}</li>)}</ul>
    </section>
  </div>
}
