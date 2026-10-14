<script lang="ts">
  import { formatSpeed } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import Icon from "./Icon.svelte";
</script>

<footer>
  <span class="num" title="Total download speed"><Icon name="down" size={13} /> {formatSpeed(torrents.totals.down)}</span>
  <span class="num" title="Total upload speed"><Icon name="up" size={13} /> {formatSpeed(torrents.totals.up)}</span>
  <span class="end num">{torrents.counts.all} {torrents.counts.all === 1 ? "torrent" : "torrents"} · {torrents.counts.downloading} active</span>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 16px;
    height: 26px;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-secondary);
  }
  span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .end {
    margin-left: auto;
    color: var(--text-muted);
  }
</style>
