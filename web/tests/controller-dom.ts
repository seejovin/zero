/** A small in-memory DOM for controller regressions; not a browser/layout substitute. */
type Listener = (event: any) => void;
class FixtureEvents {
  private readonly listeners = new Map<string, Listener[]>();
  addEventListener(type: string, listener: Listener): void { this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]); }
  dispatchEvent(event: any): boolean {
    event.target ??= this;
    event.preventDefault ??= () => { event.defaultPrevented = true; };
    for (const listener of [...(this.listeners.get(event.type) ?? [])]) listener(event);
    return !event.defaultPrevented;
  }
}

class FixtureNode extends FixtureEvents {
  parentElement: FixtureElement | null = null;
  childNodes: FixtureNode[] = [];
  constructor(readonly ownerDocument: FixtureDocument, private data = "") { super(); }
  get textContent(): string { return this.childNodes.length ? this.childNodes.map(node => node.textContent).join("") : this.data; }
  set textContent(value: string) { this.replaceChildren(); this.data = String(value); }
  append(...nodes: (FixtureNode | string)[]): void {
    for (const value of nodes) {
      const node = typeof value === "string" ? this.ownerDocument.createTextNode(value) : value;
      node.remove(); node.parentElement = this as unknown as FixtureElement; this.childNodes.push(node);
    }
  }
  replaceChildren(...nodes: (FixtureNode | string)[]): void {
    for (const node of this.childNodes) { if (node.contains(this.ownerDocument.activeElement)) this.ownerDocument.activeElement = this.ownerDocument.body; node.parentElement = null; }
    this.childNodes = []; this.data = ""; this.append(...nodes);
  }
  remove(): void {
    const parent = this.parentElement;
    if (parent) { parent.childNodes = parent.childNodes.filter(node => node !== this); this.parentElement = null; }
  }
  replaceWith(node: FixtureNode): void {
    const parent = this.parentElement;
    if (!parent) return;
    const index = parent.childNodes.indexOf(this);
    if (this.contains(this.ownerDocument.activeElement)) this.ownerDocument.activeElement = this.ownerDocument.body;
    node.remove(); parent.childNodes[index] = node; node.parentElement = parent; this.parentElement = null;
  }
  contains(node: FixtureNode | null): boolean { return node === this || this.childNodes.some(child => child.contains(node)); }
  isEqualNode(other: FixtureNode): boolean {
    return this.constructor === other.constructor && this.textContent === other.textContent
      && (!(this instanceof FixtureElement) || (other instanceof FixtureElement && this.tagName === other.tagName))
      && JSON.stringify(this instanceof FixtureElement ? [...this.attributes] : []) === JSON.stringify(other instanceof FixtureElement ? [...other.attributes] : [])
      && this.childNodes.length === other.childNodes.length && this.childNodes.every((child, index) => child.isEqualNode(other.childNodes[index]));
  }
}

export class FixtureElement extends FixtureNode {
  readonly attributes = new Map<string, string>();
  readonly dataset: Record<string, string> = {};
  readonly style: Record<string, string> = {};
  value = ""; checked = false; disabled = false; hidden = false; open = false;
  constructor(document: FixtureDocument, readonly tagName: string) { super(document); }
  get id(): string { return this.getAttribute("id") ?? ""; }
  set id(value: string) { this.setAttribute("id", value); }
  get className(): string { return this.getAttribute("class") ?? ""; }
  set className(value: string) { this.setAttribute("class", value); }
  get children(): FixtureElement[] { return this.childNodes.filter((node): node is FixtureElement => node instanceof FixtureElement); }
  get classList() {
    const write = (token: string, enabled: boolean): boolean => {
      const tokens = new Set(this.className.split(/\s+/).filter(Boolean));
      if (enabled) tokens.add(token); else tokens.delete(token);
      this.className = [...tokens].join(" "); return enabled;
    };
    return { add: (...tokens: string[]) => tokens.forEach(token => write(token, true)), remove: (...tokens: string[]) => tokens.forEach(token => write(token, false)),
      contains: (token: string) => this.className.split(/\s+/).includes(token), toggle: (token: string, force?: boolean) => write(token, force ?? !this.className.split(/\s+/).includes(token)) };
  }
  setAttribute(name: string, value: string): void {
    this.attributes.set(name, String(value));
    if (name.startsWith("data-")) this.dataset[name.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = String(value);
    if (name === "value") this.value = String(value);
  }
  getAttribute(name: string): string | null { return this.attributes.get(name) ?? null; }
  removeAttribute(name: string): void { this.attributes.delete(name); }
  matches(selector: string): boolean {
    const attrs = [...selector.matchAll(/\[([^=\]]+)(?:=["']?([^"'\]]*)["']?)?\]/g)];
    const plain = selector.replace(/\[[^\]]*\]/g, "");
    const tag = plain.match(/^[a-zA-Z][\w-]*/)?.[0];
    if (tag && tag.toLowerCase() !== this.tagName.toLowerCase()) return false;
    const id = plain.match(/#([\w-]+)/)?.[1];
    if (id && this.id !== id) return false;
    if ([...plain.matchAll(/\.([\w-]+)/g)].some(match => !this.classList.contains(match[1]))) return false;
    return attrs.every(([, name, value]) => this.attributes.has(name) && (value === undefined || this.getAttribute(name) === value));
  }
  querySelectorAll(selector: string): FixtureElement[] {
    const selectors = selector.split(",").map(value => value.trim());
    const result: FixtureElement[] = [];
    const visit = (node: FixtureNode) => { for (const child of node.childNodes) { if (child instanceof FixtureElement && selectors.some(value => child.matches(value))) result.push(child); visit(child); } };
    visit(this); return result;
  }
  querySelector(selector: string): FixtureElement | null { return this.querySelectorAll(selector)[0] ?? null; }
  focus(): void { this.ownerDocument.activeElement = this; }
  click(): void { if (!this.disabled) this.dispatchEvent({ type: "click" }); }
  reportValidity(): boolean {
    return this.querySelectorAll("input,textarea,select").every(input => !input.attributes.has("required") || (input.getAttribute("type") === "checkbox" ? input.checked : Boolean(input.value)));
  }
}
class FixtureInput extends FixtureElement {}
class FixtureSelect extends FixtureElement {}
class FixtureTextarea extends FixtureElement {}

export class FixtureDocument extends FixtureEvents {
  readonly body = new FixtureElement(this, "body");
  activeElement: FixtureNode = this.body;
  visibilityState = "visible";
  createElement(tag: string): FixtureElement {
    return tag === "input" ? new FixtureInput(this, tag) : tag === "select" ? new FixtureSelect(this, tag) : tag === "textarea" ? new FixtureTextarea(this, tag) : new FixtureElement(this, tag);
  }
  createElementNS(_namespace: string, tag: string): FixtureElement { return this.createElement(tag); }
  createTextNode(text: string): FixtureNode { return new FixtureNode(this, text); }
  getElementById(id: string): FixtureElement | null { return this.body.querySelector(`#${id}`); }
  querySelectorAll(selector: string): FixtureElement[] { return this.body.querySelectorAll(selector); }
}

export function fixtureBrowser() {
  const document = new FixtureDocument();
  const mount = document.createElement("div"); mount.id = "app"; document.body.append(mount);
  const window = Object.assign(new FixtureEvents(), { scrollTo() {}, top: null as unknown, self: null as unknown });
  window.top = window; window.self = window;
  let timerId = 0;
  const timers = new Map<number, { callback: () => void; delay: number }>();
  const storage = new Map<string, string>();
  const qr: string[] = [];
  const globals = {
    document, window, HTMLElement: FixtureElement, HTMLInputElement: FixtureInput, HTMLSelectElement: FixtureSelect,
    HTMLTextAreaElement: FixtureTextarea, HTMLButtonElement: FixtureElement, Node: FixtureNode,
    navigator: { clipboard: { writeText: async (_value: string) => {} }, storage: { persist: async () => false } },
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => { storage.set(key, value); } },
    isSecureContext: true, indexedDB: {}, Worker: class {}, BroadcastChannel: undefined,
    crypto: { subtle: {}, getRandomValues() { throw new Error("Controller fixtures must not generate keys."); } },
    AbortController,
    setTimeout: (callback: () => void, delay = 0) => { const id = ++timerId; timers.set(id, { callback, delay }); return id; },
    clearTimeout: (id: number) => timers.delete(id), setInterval: () => ++timerId,
    fetch() { throw new Error("Controller fixtures must not make network requests."); },
  };
  return { globals, document, mount, storage, qr, flushTimers(delay: number) {
    for (const [id, timer] of [...timers]) if (timer.delay === delay) { timers.delete(id); timer.callback(); }
  } };
}
