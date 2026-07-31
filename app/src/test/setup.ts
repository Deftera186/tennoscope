import '@testing-library/jest-dom/vitest'

// Node 26 defines its own `localStorage` global, which is undefined unless the process was started
// with `--localstorage-file`, and it shadows the working one jsdom builds for every test window.
// `sessionStorage` comes through untouched, so this is a name collision rather than a missing
// feature -- but the preference under test is one that has to outlive the window, so the app asks
// for the storage that does. This stands a plain one in its place; jsdom is torn down per file, so
// it is as isolated as the real thing.
if (typeof localStorage === 'undefined') {
  const store = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      get length() { return store.size },
      key: (index: number) => [...store.keys()][index] ?? null,
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)) },
      removeItem: (key: string) => { store.delete(key) },
      clear: () => store.clear(),
    } satisfies Storage,
  })
}
