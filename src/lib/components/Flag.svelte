<script lang="ts">
  // Windows has no flag emoji, so flags are SVG files (flag-icons, MIT), loaded one at a time when first shown.
  const files = import.meta.glob("/node_modules/flag-icons/flags/4x3/*.svg", { query: "?url", import: "default" });
  const names = new Intl.DisplayNames(undefined, { type: "region" });

  let { code }: { code: string | null } = $props();

  let src = $state("");
  const name = $derived.by(() => {
    try {
      return code ? (names.of(code.toUpperCase()) ?? code) : "";
    } catch {
      return code ?? "";
    }
  });

  $effect(() => {
    const load = code ? files[`/node_modules/flag-icons/flags/4x3/${code.toLowerCase()}.svg`] : undefined;
    src = "";
    if (load) void load().then((u) => (src = u as string));
  });
</script>

{#if src}
  <img {src} alt={name} title={name} width="16" height="12" />
{:else}
  <span class="none" title={name || "Unknown country"} aria-label={name || "Unknown country"}>{code ?? ""}</span>
{/if}

<style>
  img {
    display: block;
    border-radius: 2px;
    box-shadow: 0 0 0 1px var(--border);
  }
  .none {
    font-size: 10px;
    color: var(--text-muted);
    width: 16px;
    text-align: center;
  }
</style>
