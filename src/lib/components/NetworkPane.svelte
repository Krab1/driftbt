<script lang="ts">
  import { t, td } from "../i18n.svelte";
  import { onMount } from "svelte";
  import { api, type Adapter, type FilterStatus, type NetworkStatus } from "../api";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const s = $derived(ui.settings!);

  let net = $state<NetworkStatus | null>(null);
  let adapters = $state<Adapter[]>([]);
  let filter = $state<FilterStatus | null>(null);
  let busy = $state("");
  let proxyText = $state(ui.settings?.proxy ?? "");
  let newName = $state("");
  let newSource = $state("");
  let listError = $state("");

  async function load() {
    net = await api.networkStatus().catch(() => null);
    filter = await api.filterStatus().catch(() => null);
  }
  async function loadAdapters() {
    adapters = await api.listAdapters().catch(() => []);
  }
  onMount(() => {
    void load();
    void loadAdapters();
  });

  /** Network changes restart the engine's session, which takes a moment. */
  async function apply(label: string, patch: Parameters<typeof ui.saveSettings>[0]) {
    busy = label;
    const ok = await ui.saveSettings(patch);
    busy = "";
    if (ok) ui.toast(t("Network settings applied"));
    await load();
  }

  function port(e: Event): number | null {
    const n = Math.floor(Number((e.currentTarget as HTMLInputElement).value));
    return Number.isFinite(n) && n >= 1024 && n <= 65535 ? n : null;
  }

  const age = (secs: number) => {
    const d = Math.max(0, Math.floor(Date.now() / 1000) - secs);
    return d < 90 ? t("just now") : d < 5400 ? t("{n} min ago", { n: Math.round(d / 60) }) : d < 172800 ? t("{n} h ago", { n: Math.round(d / 3600) }) : t("{n} days ago", { n: Math.round(d / 86400) });
  };

  const slug = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40) || "list";

  async function addList(name: string, source: string) {
    const f = s.ip_filter;
    name = name.trim();
    source = source.trim();
    if (!source) return void (listError = t("Enter an address or choose a file."));
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(source) && !/^https?:\/\//i.test(source)) return void (listError = "Only http and https addresses work.");
    if (f.lists.some((l) => l.source === source)) return void (listError = t("That list is already added."));
    listError = "";
    let id = slug(name || source.split(/[\\/]/).pop() || "list");
    for (let n = 2; f.lists.some((l) => l.id === id); n++) id = `${slug(name || "list")}-${n}`;
    busy = "list";
    await apply("list", { ip_filter: { enabled: true, lists: [...f.lists, { id, name: name || id, source, enabled: true }] } });
    newName = newSource = "";
  }

  async function chooseFile() {
    const p = await api.pickFile(null);
    if (p) newSource = p;
  }

  async function updateNow() {
    busy = "update";
    try {
      filter = await api.updateIpLists();
      ui.toast(t("IP lists updated"));
    } catch (e) {
      ui.toast(td((e as { message?: string }).message ?? t("Couldn't update the lists.")), "error");
    }
    busy = "";
  }

  const emule = { name: "eMule Security IP filter", source: "http://upd.emule-security.org/ipfilter.zip" };
</script>

{#key ui.settingsRev}
  <div class="field">
    <span>{t("Incoming connections")}</span>
    <label class="inline">{t("Port")} <input type="number" min="1024" max="65535" inputmode="numeric" value={s.listen_port ?? ""} placeholder={t("Random")} disabled={!!busy} onchange={(e) => apply("port", { listen_port: port(e) })} /></label>
    <label class="check"><input type="checkbox" checked={s.upnp} disabled={!!busy || !!s.proxy} onchange={(e) => apply("upnp", { upnp: e.currentTarget.checked })} /> {t("Open the port on my router automatically (UPnP)")}</label>
    <label class="check"><input type="checkbox" checked={s.dht} disabled={!!busy || !!s.proxy} onchange={(e) => apply("dht", { dht: e.currentTarget.checked })} /> {t("Find more peers through DHT")}</label>
    <small role="status">
      {#if busy}{t("Applying… torrents pause for a moment.")}
      {:else if net?.listen_port}{t("Listening on port")} <b class="num">{net.listen_port}</b>.
      {:else if s.proxy}{t("Not listening: a proxy is set, so incoming connections and DHT are off.")}
      {:else}{t("Not listening right now.")}{/if}
      {t("A fixed port must be at least 1024; leave it empty to pick a random one at each start.")}
    </small>
  </div>

  <div class="field">
    <span>{t("Proxy")}</span>
    <div class="row">
      <input class="txt" type="text" placeholder={t("socks5://127.0.0.1:1080")} aria-label={t("SOCKS5 proxy address")} bind:value={proxyText} disabled={!!busy} onkeydown={(e) => e.key === "Enter" && apply("proxy", { proxy: proxyText.trim() || null })} />
      <button class="btn" disabled={!!busy} onclick={() => apply("proxy", { proxy: proxyText.trim() || null })}>{s.proxy ? t("Update") : t("Use proxy")}</button>
      {#if s.proxy}<button class="btn" disabled={!!busy} onclick={() => { proxyText = ""; apply("proxy", { proxy: null }); }}>{t("Remove")}</button>{/if}
    </div>
    <small>{t("Outgoing connections go through the proxy. Incoming connections and DHT are switched off while it is set, so your real address isn't exposed around it.")}</small>
  </div>

  <div class="field">
    <span>{t("VPN guard")}</span>
    <label class="check"><input type="checkbox" checked={s.vpn_guard.enabled} onchange={(e) => ui.saveSettings({ vpn_guard: { ...s.vpn_guard, enabled: e.currentTarget.checked } })} /> {t("Pause all torrents when my VPN isn't connected")}</label>
    <div class="row">
      <select aria-label={t("VPN adapter")} value={s.vpn_guard.adapter ?? ""} onchange={(e) => ui.saveSettings({ vpn_guard: { ...s.vpn_guard, adapter: e.currentTarget.value || null } })}>
        <option value="">{t("Choose your VPN adapter…")}</option>
        {#each adapters as a (a.name)}
          <option value={a.name}>{a.name}{a.up && a.has_ipv4 ? "" : t(" (not connected)")} — {a.description}</option>
        {/each}
        {#if s.vpn_guard.adapter && !adapters.some((a) => a.name === s.vpn_guard.adapter)}<option value={s.vpn_guard.adapter}>{t("{name} (not found)", { name: s.vpn_guard.adapter })}</option>{/if}
      </select>
      <button class="btn" onclick={loadAdapters}>{t("Refresh")}</button>
    </div>
    <label class="check"><input type="checkbox" checked={s.vpn_guard.auto_resume} onchange={(e) => ui.saveSettings({ vpn_guard: { ...s.vpn_guard, auto_resume: e.currentTarget.checked } })} /> {t("Resume what it paused when the VPN comes back")}</label>
    <small>
      {t("Drift can't bind to an adapter on Windows, so this is a watchdog: within a few seconds of your VPN dropping it pauses everything and holds the queue. A little traffic could leave in that gap. For a hard guarantee, use your VPN's own kill switch as well.")}
    </small>
  </div>

  <div class="field">
    <span>{t("IP filter")}</span>
    <label class="check"><input type="checkbox" checked={s.ip_filter.enabled} disabled={!!busy} onchange={(e) => apply("filter", { ip_filter: { ...s.ip_filter, enabled: e.currentTarget.checked } })} /> {t("Block connections to and from addresses on these lists")}</label>

    {#each s.ip_filter.lists as l (l.id)}
      {@const st = filter?.lists[l.id]}
      <div class="list">
        <input type="checkbox" aria-label="Use {l.name}" checked={l.enabled} disabled={!!busy} onchange={(e) => apply("list", { ip_filter: { ...s.ip_filter, lists: s.ip_filter.lists.map((x) => (x.id === l.id ? { ...x, enabled: e.currentTarget.checked } : x)) } })} />
        <div class="meta">
          <div class="nm"><Icon name="file" size={14} />{l.name}</div>
          <code title={l.source}>{l.source}</code>
          <small>
            {#if st?.updated_at}{t("{n} ranges · updated {age}", { n: st.ranges.toLocaleString(), age: age(st.updated_at) })}{:else}{t("Not downloaded yet")}{/if}
            {#if st?.insecure} · <span class="warn">{t("plain http: it could be altered on the way")}</span>{/if}
          </small>
          {#if st?.error}<small class="bad" role="alert">{st.error}{st.updated_at ? t(" The previous copy is still in use.") : ""}</small>{/if}
        </div>
        <button class="btn ghost icon" aria-label="Remove {l.name}" disabled={!!busy} onclick={() => apply("list", { ip_filter: { ...s.ip_filter, lists: s.ip_filter.lists.filter((x) => x.id !== l.id) } })}><Icon name="x" size={14} /></button>
      </div>
    {/each}

    <div class="row">
      <input class="txt" type="text" placeholder={t("Name (optional)")} aria-label={t("List name")} bind:value={newName} />
      <input class="txt wide" type="text" placeholder={t("https://… or a file path")} aria-label={t("List address or file")} bind:value={newSource} onkeydown={(e) => e.key === "Enter" && addList(newName, newSource)} />
      <button class="btn" onclick={chooseFile}><Icon name="folder" />{t("File…")}</button>
      <button class="btn" disabled={!!busy} onclick={() => addList(newName, newSource)}><Icon name="plus" />{t("Add")}</button>
    </div>
    {#if listError}<small class="bad" role="alert">{listError}</small>{/if}

    <div class="row">
      {#if !s.ip_filter.lists.some((l) => l.source === emule.source)}
        <button class="btn" disabled={!!busy} onclick={() => addList(emule.name, emule.source)}>{t("Add suggested: {name}", { name: emule.name })}</button>
      {/if}
      <button class="btn" disabled={!!busy || !s.ip_filter.lists.length} onclick={updateNow}>{busy === "update" ? t("Updating…") : t("Update now")}</button>
      {#if filter?.enabled}<small class="num">{t("{n} ranges active", { n: filter.active_ranges.toLocaleString() })}</small>{/if}
    </div>
    <small>
      {t("Understands eMule ipfilter.dat, PeerGuardian and plain range lists, also zipped or gzipped. Lists are checked before use: empty ones, huge ones, and ones that would block more than a quarter of the internet are ignored. This blocks connections; it doesn't hide your address, so it isn't a substitute for a VPN. Lists refresh weekly, and the engine briefly restarts when one changes.")}
    </small>
  </div>
{/key}

<style>
  .field {
    display: grid;
    gap: 8px;
  }
  .field > span {
    color: var(--text-secondary);
    font-size: 12px;
  }
  small {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger-fg);
  }
  .warn {
    color: var(--warning);
  }
  .check,
  .inline {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
  }
  input[type="number"],
  .txt,
  select {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    user-select: text;
  }
  input[type="number"] {
    width: 96px;
    text-align: right;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .txt {
    flex: 1;
    min-width: 120px;
  }
  .txt.wide {
    flex: 2;
    min-width: 200px;
  }
  select {
    flex: 1;
    min-width: 0;
  }
  .list {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .list > input {
    margin-top: 4px;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .nm {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
