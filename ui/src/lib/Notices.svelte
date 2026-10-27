<script lang="ts" module>
  export interface Notice {
    /// Stable for as long as it is the same notice, so it is not redrawn (and
    /// does not slide in again) on every state change around it.
    key: string;
    text: string;
    /// Offered a close button. A notice about something still true — the
    /// remote page's lost connection — is not: it goes when the cause does.
    dismiss?: () => void;
  }
</script>

<script lang="ts">
  // What has gone wrong, over the top of the chat.
  //
  // Only errors come here. The session's ordinary states — loading, running,
  // ended — are on the status dot beside the app's name and nowhere else; a
  // bar that is always there is a bar nobody reads, so this one is there only
  // when it has something to say.
  //
  // Over the chat rather than pushing it down: a notice that arrives during a
  // show must not move the line the operator was reading. Below the header
  // rather than over it, because the header holds Start, and a failed start
  // is answered by pressing Start again.

  import { fly } from "svelte/transition";

  interface Props {
    notices: Notice[];
  }

  let { notices }: Props = $props();
</script>

{#if notices.length > 0}
  <div class="notices" role="alert">
    {#each notices as notice (notice.key)}
      <div class="notice" transition:fly={{ y: -12, duration: 160 }}>
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <path d="M8 1.6 15 14H1z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
          <path d="M8 6v3.6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
          <circle cx="8" cy="11.8" r="0.9" fill="currentColor" />
        </svg>
        <span class="text">{notice.text}</span>
        {#if notice.dismiss}
          <button type="button" onclick={notice.dismiss} aria-label="Dismiss" title="Dismiss">
            <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
              <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
            </svg>
          </button>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .notices {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    /* Over the sidebar's drawer as well: an error is about the whole window. */
    z-index: 25;
    display: flex;
    flex-direction: column;
    pointer-events: none;
  }

  .notice {
    display: flex;
    align-items: flex-start;
    gap: 0.55rem;
    padding: 0.5rem 1rem;
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(0.5rem, var(--safe-right));
    font-size: 0.85rem;
    line-height: 1.4;
    color: #ffd9d9;
    background: #4a2222;
    border-bottom: 1px solid #6b2f2f;
    box-shadow: 0 0.3rem 0.9rem rgb(0 0 0 / 0.35);
    pointer-events: auto;
  }

  svg {
    flex: none;
  }

  .notice > svg {
    margin-top: 0.15rem;
    color: #ff6b6b;
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  button {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 1.5rem;
    height: 1.5rem;
    margin: -0.1rem 0;
    padding: 0;
    color: inherit;
    background: transparent;
    border: none;
    border-radius: 0.3rem;
    opacity: 0.75;
    cursor: pointer;
  }

  button:hover {
    opacity: 1;
    background: rgb(255 255 255 / 0.08);
  }

  @media (max-width: 45rem) {
    .notice {
      padding: 0.4rem 0.6rem;
      padding-left: max(0.6rem, var(--safe-left));
      padding-right: max(0.4rem, var(--safe-right));
      font-size: 0.8rem;
    }
  }
</style>
