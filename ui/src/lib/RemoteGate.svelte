<script lang="ts">
  // What a remote page renders before it has been let in (P16).
  //
  // Around the view rather than inside it, because there are two views — the
  // main window and a detached group's — and both are served to a browser.
  // A gate in one of them would be a gate the other does not have.
  //
  // Nothing at all while the check is in flight: it is one request on a local
  // network, and mounting the app for those few milliseconds would start the
  // calls it makes when it opens, which would then be queued against a socket
  // that is never going to open and time out half a minute later. In the app
  // the lock is `"open"` from the first line of `transport.ts`, so this renders
  // its view immediately and is never seen.

  import type { Component } from "svelte";
  import RemoteKey from "./RemoteKey.svelte";
  import { lockState, onLock, type Lock } from "./transport";

  interface Props {
    /// The view this page would be showing.
    view: Component<any>;
    props?: Record<string, unknown>;
  }

  let { view, props = {} }: Props = $props();

  let lock = $state<Lock>(lockState());
  // The unsubscribe is the effect's cleanup, which is what stops a detached
  // window's gate from being called back after the window has gone.
  $effect(() => onLock((state) => (lock = state)));

  const View = $derived(view);
</script>

{#if lock === "needed"}
  <RemoteKey />
{:else if lock === "open"}
  <View {...props} />
{/if}
