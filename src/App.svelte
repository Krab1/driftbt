<script lang="ts">
  import { onMount } from "svelte";
  import Login from "./lib/components/Login.svelte";
  import Main from "./Main.svelte";
  import { auth } from "./lib/stores/auth.svelte";

  onMount(() => void auth.start());
</script>

{#if auth.state === "in"}
  <Main />
{:else if auth.state === "out"}
  <Login />
{/if}
