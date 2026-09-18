<script lang="ts">
  import { tick } from "svelte";
  import type { TranscriptEvent } from "./bridge";
  import { toBubbles, type Bubble } from "./bubbles";

  /// How many bubbles are in the DOM at once, and how many more each step back
  /// through the history adds.
  ///
  /// An hour across eight streams is tens of thousands of nodes, and a browser
  /// that holds all of them scrolls in steps. So the list renders a window at
  /// the end of the chat — which is where a live view is read — and extends it
  /// backwards when the operator scrolls up to the top of it.
  ///
  /// A window, not a virtual scroller with estimated heights: a bubble's
  /// height depends on how its text wraps, so estimating it means a scrollbar
  /// that lies and jumps as the estimates are corrected. What a window costs
  /// instead is that reaching a point an hour back takes several steps — which
  /// is what P9's search and its jump into the chat position are for, rather
  /// than scrolling.
  const PAGE = 200;

  interface Props {
    messages: TranscriptEvent[];
    /// Channel colour per stream id. A line belongs to a channel, and with
    /// eight of them interleaved the colour is what makes that readable at a
    /// glance — faster than reading eight labels.
    colors: Map<number, string>;
    /// When each session's clock was zero, in epoch milliseconds. Per session
    /// rather than one value, because the chat holds lines from every session
    /// that has run since it was cleared.
    starts: Map<number, number>;
  }

  let { messages, colors, starts }: Props = $props();

  // Consecutive segments of the same channel are one bubble; see
  // [`toBubbles`] for where a bubble ends.
  let bubbles = $derived(toBubbles(messages));

  let limit = $state(PAGE);
  let shown = $derived(bubbles.slice(Math.max(0, bubbles.length - limit)));
  let hidden = $derived(bubbles.length - shown.length);

  const colorOf = (streamId: number) => colors.get(streamId) ?? "var(--accent)";

  let container: HTMLDivElement | undefined = $state();
  // Follow behaviour: stay pinned to the bottom, but not while the user is
  // reading further up. Still to come in P7: a proper "N new messages" button.
  let following = $state(true);

  /// Extends the window backwards, keeping what is on screen where it is.
  ///
  /// Content added *above* the viewport pushes everything down by its own
  /// height, so the line the operator was reading would jump off the screen.
  /// The scroll position is therefore corrected by however much the content
  /// grew, measured across the DOM update rather than guessed.
  const showMore = async () => {
    if (!container || hidden === 0) return;

    const before = container.scrollHeight;
    limit += PAGE;
    await tick();
    if (container) container.scrollTop += container.scrollHeight - before;
  };

  const onScroll = () => {
    if (!container) return;
    const distance =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    following = distance < 80;
    // Near the top of the window and there is more behind it: reach for the
    // next page before the operator hits the end of what is rendered, so the
    // history reads as one list rather than as pages.
    //
    // Only while reading back. At the bottom the follow effect trims the
    // window again, and a short chat sits at `scrollTop` 0 with both
    // conditions true — growing and trimming each other in a loop.
    if (!following && container.scrollTop < 400) void showMore();
  };

  $effect(() => {
    // Touch the message count, not the bubble count: a segment appended to
    // the last bubble leaves the number of bubbles unchanged while the box
    // still grows, and the view has to follow that too.
    messages.length;
    if (following && container) {
      // Back at the bottom, the pages that were opened to read backwards are
      // no longer being looked at. Dropping them keeps a session that ran for
      // hours from carrying every node it ever rendered — and it is invisible,
      // because everything dropped is above the viewport.
      limit = PAGE;
      container.scrollTop = container.scrollHeight;
    }
  });

  const latencyTitle = (bubble: Bubble) =>
    bubble.parts === 1
      ? `Decode ${bubble.decodeMs} ms of ${bubble.latencyMs} ms total`
      : `Decode ${bubble.decodeMs} ms of ${bubble.latencyMs} ms total, last of ${bubble.parts} segments`;

  const pad = (n: number) => String(n).padStart(2, "0");

  /// How old a line may be before it is given its time of day instead.
  ///
  /// Inside the hour, "4 minutes ago" is the question actually being asked of
  /// a live chat — how far behind the room the text is. Past it, that reading
  /// stops working ("73 minutes ago" is arithmetic, not a time) and a clock
  /// that can be matched against a running order is worth more.
  const RELATIVE_MS = 60 * 60 * 1000;

  /// Under this, the line is "now": inside a minute there is nothing to say
  /// about how old it is, and a counter ticking up through the seconds draws
  /// the eye away from the text.
  const NOW_MS = 45_000;

  // Fixed to English, like the rest of the interface. The system locale would
  // otherwise put a German "vor 4 Minuten" next to an English "Jump to
  // latest".
  const relative = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

  // Re-read every half minute so the ages age. Nothing else moves the clock
  // forward: a bubble that is not touched again would otherwise keep saying
  // "now" for the rest of the session.
  let now = $state(Date.now());

  $effect(() => {
    const tick = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(tick);
  });

  /// When a bubble started, in epoch milliseconds, or `null` when its
  /// session's zero point is not known.
  const startedAt = (bubble: Bubble) => {
    const start = starts.get(bubble.sessionId);
    return start === undefined ? null : start + bubble.tStartMs;
  };

  /// What the header shows: how long ago for a fresh line, the time of day for
  /// an older one.
  ///
  /// Falls back to time since the session started when the zero point is not
  /// known — a chat that outlived the state that carried it. Marked with a
  /// leading `+` so it is never read as a time of day.
  const clock = (bubble: Bubble) => {
    const at = startedAt(bubble);
    if (at === null) {
      const total = Math.floor(bubble.tStartMs / 1000);
      return `+${Math.floor(total / 60)}:${pad(total % 60)}`;
    }

    // A machine whose clock stepped backwards can put a line in the future.
    // Zero rather than a negative age, which would read as "in 3 minutes".
    const age = Math.max(0, now - at);
    if (age >= RELATIVE_MS) {
      const time = new Date(at);
      return `${pad(time.getHours())}:${pad(time.getMinutes())}`;
    }
    if (age < NOW_MS) return relative.format(0, "second");
    // Floor, not round: a line stays "1 minute ago" until it really is two
    // minutes old. Rounding would call a 90-second-old line two minutes,
    // which is the one direction a latency figure must not err in.
    return relative.format(-Math.floor(age / 60_000), "minute");
  };

  /// The exact time of day, for the line the operator stops to look at. The
  /// seconds are here rather than in the header: they matter when a line is
  /// being placed against something else, and never while reading the chat.
  const clockTitle = (bubble: Bubble) => {
    const at = startedAt(bubble);
    if (at === null) return "Time of day unknown for this session";
    const time = new Date(at);
    return `${pad(time.getHours())}:${pad(time.getMinutes())}:${pad(time.getSeconds())}`;
  };
</script>

<div class="chat" bind:this={container} onscroll={onScroll}>
  {#if messages.length === 0}
    <p class="empty">Nothing transcribed yet. Speak into one of the channels.</p>
  {/if}

  {#if hidden > 0}
    <button class="earlier" onclick={showMore}>
      {hidden} earlier {hidden === 1 ? "bubble" : "bubbles"} — show more
    </button>
  {/if}

  <!-- Keyed on the identity of the segment that opened the bubble. On all
       three parts, not on `streamId + seq`: those repeat across sessions, and
       a duplicate key throws and stops the list rendering entirely — which is
       what "nothing appears after Stop and Start" looked like. -->
  {#each shown as bubble (bubble.key)}
    <article style="--channel: {colorOf(bubble.streamId)}">
      <header>
        <span class="label">{bubble.label || "Stream " + bubble.streamId}</span>
        <span class="time" title={clockTitle(bubble)}>{clock(bubble)}</span>
        <span class="latency" title={latencyTitle(bubble)}>
          {(bubble.latencyMs / 1000).toFixed(1)} s
        </span>
      </header>
      <p>{bubble.text}</p>
    </article>
  {/each}
</div>

{#if !following}
  <button
    class="jump"
    onclick={() => {
      following = true;
      if (container) container.scrollTop = container.scrollHeight;
    }}
  >
    Jump to latest
  </button>
{/if}

<style>
  .chat {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .empty {
    color: var(--text-dim);
    margin: auto;
  }

  .earlier {
    align-self: center;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 1rem;
    padding: 0.25rem 0.8rem;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .earlier:hover {
    color: var(--text);
  }

  article {
    background: var(--surface);
    border: 1px solid var(--border);
    /* The channel's colour down the left edge: with eight channels
       interleaved this is what groups the lines, and it survives being read
       out of the corner of an eye while doing something else. */
    border-left: 3px solid var(--channel);
    border-radius: 0.5rem;
    padding: 0.5rem 0.75rem;
    max-width: 48rem;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    margin-bottom: 0.15rem;
  }

  .label {
    color: var(--channel);
    font-weight: 600;
  }

  .time,
  .latency {
    font-variant-numeric: tabular-nums;
  }

  .latency {
    margin-left: auto;
  }

  article p {
    margin: 0;
  }

  .jump {
    position: absolute;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    background: var(--accent);
    color: #12141a;
    border: none;
    border-radius: 1rem;
    padding: 0.35rem 0.9rem;
    cursor: pointer;
    font-weight: 600;
  }
</style>
