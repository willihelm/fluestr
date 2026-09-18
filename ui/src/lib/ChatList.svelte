<script lang="ts">
  import { tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { TranscriptEvent } from "./bridge";
  import { toBubbles, type Bubble } from "./bubbles";
  import { findAll, matches, split } from "./search";

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
    /// The channels this view shows, in stream order, for the channel filter
    /// (P9). Handed down rather than derived from the bubbles: a channel that
    /// has not spoken yet is still one an operator may want to sit on, and a
    /// picker whose entries appear as people talk is a picker that moves under
    /// the cursor.
    streams: { streamId: number; label: string }[];
  }

  let { messages, colors, starts, streams }: Props = $props();

  // --- Search and filter (P9) ---------------------------------------------
  //
  // All of it local to this component, and that is the design rather than an
  // implementation detail: every view onto the chat mounts its own, so the
  // operator at the desk, the one at a detached group window and every browser
  // on the LAN (P16) search independently. Nothing here is emitted, saved or
  // sent anywhere — see `search.ts`.
  //
  // It also means the feature comes with the component: a view that renders
  // the chat can search it, and there is no second place to add it to.

  let query = $state("");
  /// Only the lines that match, rather than the whole chat with the hits
  /// marked. The two readings of one box — "did anyone mention this" wants the
  /// chat around the answer, "let me watch for this" wants only the answer —
  /// and an operator switches between them mid-question.
  let onlyMatches = $state(false);
  /// One channel of this view, or `null` for all of them.
  let stream = $state<number | null>(null);
  /// Which hit the ↑/↓ buttons are on, as a position in `hits`.
  let focus = $state(0);

  let searching = $derived(query.trim() !== "");

  // Consecutive segments of the same channel are one bubble; see
  // [`toBubbles`] for where a bubble ends.
  let bubbles = $derived(toBubbles(messages));

  /// The chat as this view has narrowed it: one channel, and under the toggle
  /// only the lines that match.
  ///
  /// The window, the "N earlier" button and the follow-to-the-bottom effect
  /// all work on this rather than on `bubbles`, so a filtered chat scrolls and
  /// pages like any other — there is no second reading mode to maintain.
  let listed = $derived(
    bubbles.filter(
      (b) =>
        (stream === null || b.streamId === stream) &&
        (!onlyMatches || !searching || matches(b.text, query)),
    ),
  );

  /// Where the hits are, as positions in `listed`.
  ///
  /// By line and not by occurrence: ↑ and ↓ move the view, and two hits in one
  /// bubble are one place to look. The count says "lines" so it cannot be
  /// misread as the number of times the word was said.
  let hits = $derived(
    searching
      ? listed.flatMap((b, index) => (matches(b.text, query) ? [index] : []))
      : [],
  );

  let limit = $state(PAGE);
  let shown = $derived(listed.slice(Math.max(0, listed.length - limit)));
  let hidden = $derived(listed.length - shown.length);
  /// The bubble the ↑/↓ buttons are sitting on, by key, or `null`.
  let focused = $derived(searching ? (listed[hits[focus]]?.key ?? null) : null);

  const colorOf = (streamId: number) => colors.get(streamId) ?? "var(--accent)";

  // Bubbles that have just arrived and are still flashing. A bubble leaves the
  // set when its animation ends, so nothing here has to know how long the
  // flash lasts — the stylesheet does, once.
  let arriving = new SvelteSet<string>();
  // The keys of the previous pass, and `null` before the first one. Plain, not
  // state: it is what the effect compares against, and making it reactive
  // would make the effect its own trigger.
  let known: Set<string> | null = null;

  // Which bubbles are new. Keys, not positions: a bubble that grows by another
  // segment keeps its key and must not flash again — one new bubble, one
  // flash, which is the same thing the attention request is counted by.
  //
  // Every view, with nothing to switch it on: it is not an attention request
  // but the answer to "which of these lines is the one that just arrived",
  // which is a question an operator watching eight interleaved channels has
  // whatever the view's settings say. What a view's flash setting adds is the
  // loud request — the whole window — and that is made outside this component.
  //
  // The first pass only takes stock. Everything on screen when the view opens
  // is history, and a chat that flashes every line of it on arrival teaches
  // the operator to ignore the flash.
  $effect(() => {
    const keys = bubbles.map((b) => b.key);
    untrack(() => {
      const previous = known;
      known = new Set(keys);
      if (previous === null) return;
      for (const key of keys) {
        if (!previous.has(key)) arriving.add(key);
      }
    });
  });

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

  /// Brings the hit at `focus` onto the screen (P9).
  ///
  /// The window is the reason this is not one line. The list renders the last
  /// `limit` bubbles, and a hit an hour back is simply not in the DOM — so the
  /// window is first grown far enough to contain it, rounded up to whole pages
  /// so that repeated jumps backwards do not re-render on every step. This is
  /// what the window was always going to need: reaching a point an hour back by
  /// scrolling takes several steps, and jumping to it takes none.
  const reveal = async () => {
    const index = hits[focus];
    if (index === undefined || !container) return;

    // Landing on a line in the middle of the history is the operator reading
    // back, so the view stops chasing the bottom. They get it back with "Jump
    // to latest", which is the same way they get it back from scrolling up.
    following = false;

    const needed = listed.length - index;
    if (needed > limit) limit = Math.ceil(needed / PAGE) * PAGE;
    await tick();

    // `block: "center"` rather than "nearest": a hit scrolled to the very edge
    // of the viewport is found and then has to be found again.
    container
      ?.querySelector('[data-focused="true"]')
      ?.scrollIntoView({ block: "center" });
  };

  /// Steps to the next or previous line that matches, wrapping round.
  ///
  /// Wrapping rather than stopping at the end: the count beside the buttons
  /// says how many there are, so an operator who passes the last one has been
  /// told where they are, and being unable to get back to the first without
  /// retyping the query would be the worse of the two surprises.
  const jump = (delta: number) => {
    if (hits.length === 0) return;
    focus = (focus + delta + hits.length) % hits.length;
    void reveal();
  };

  // A new query starts at the *last* hit — the most recent thing said that
  // matches. "Did anyone mention the fly bar" is nearly always asking about
  // this evening rather than about the top of the chat, and ↑ from there walks
  // backwards through the history, which is the direction the question runs in.
  //
  // Only when the query itself changed. Recomputing on every arriving line
  // would drag the view back to a hit each time somebody spoke, which is the
  // one thing a search must not do to a chat that is still filling.
  let searched: string | null = null;
  $effect(() => {
    const current = query;
    untrack(() => {
      if (current === searched) return;
      searched = current;
      focus = Math.max(0, hits.length - 1);
      if (hits.length > 0) void reveal();
    });
  });

  // A hit that scrolled out of reach — the chat was cleared, the filter
  // narrowed — must not leave the buttons pointing past the end of the list.
  $effect(() => {
    if (focus >= hits.length) focus = Math.max(0, hits.length - 1);
  });

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

<!-- The search bar belongs to this view and to no other (P9). Above the list
     rather than in the window header, because the header is Start, Stop and
     the session — things that act on the whole app — and this acts on what is
     below it. -->
<div class="search">
  <!-- Enter steps to the next match and Shift+Enter to the previous one, which
       is what a search box does everywhere else; Escape clears it and gives
       the chat back. Without them the only way to walk a search is to take a
       hand off the keyboard for two small buttons, in a room where the other
       hand is on a fader. -->
  <input
    type="search"
    class="query"
    placeholder="Search this view"
    aria-label="Search the chat"
    bind:value={query}
    onkeydown={(e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        jump(e.shiftKey ? -1 : 1);
      } else if (e.key === "Escape") {
        query = "";
      }
    }}
  />

  {#if streams.length > 1}
    <!-- Only when there is a choice to make. On a single-channel view a picker
         reading "All channels" over one channel is furniture. -->
    <select class="channel" aria-label="Limit to one channel" bind:value={stream}>
      <option value={null}>All channels</option>
      {#each streams as option (option.streamId)}
        <option value={option.streamId}>{option.label || "Stream " + option.streamId}</option>
      {/each}
    </select>
  {/if}

  {#if searching}
    <span class="count" aria-live="polite">
      {hits.length === 0
        ? "no matches"
        : `${focus + 1} of ${hits.length} ${hits.length === 1 ? "line" : "lines"}`}
    </span>

    <button
      class="step"
      onclick={() => jump(-1)}
      disabled={hits.length === 0}
      title="Previous match">↑</button
    >
    <button
      class="step"
      onclick={() => jump(1)}
      disabled={hits.length === 0}
      title="Next match">↓</button
    >

    <label class="only">
      <input type="checkbox" bind:checked={onlyMatches} />
      Only matches
    </label>
  {/if}
</div>

<div class="chat" bind:this={container} onscroll={onScroll}>
  {#if messages.length === 0}
    <p class="empty">Nothing transcribed yet. Speak into one of the channels.</p>
  {:else if listed.length === 0}
    <!-- The chat is not empty, the view is — which is a different thing to be
         told, because it has a cause the operator can undo. -->
    <p class="empty">
      Nothing here matches. Clear the search or widen it to see the chat again.
    </p>
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
    <article
      class:arriving={arriving.has(bubble.key)}
      class:focused={bubble.key === focused}
      data-focused={bubble.key === focused}
      onanimationend={() => arriving.delete(bubble.key)}
      style="--channel: {colorOf(bubble.streamId)}"
    >
      <header>
        <span class="label">{bubble.label || "Stream " + bubble.streamId}</span>
        <span class="time" title={clockTitle(bubble)}>{clock(bubble)}</span>
        <span class="latency" title={latencyTitle(bubble)}>
          {(bubble.latencyMs / 1000).toFixed(1)} s
        </span>
      </header>
      <!-- The alert terms, marked where the backend found them (P17). Written
           out piece by piece rather than through `{@html}`: a transcript is
           text from outside, and the one thing this view must never do is let
           a spoken sentence become markup.

           On one line on purpose — Svelte keeps the whitespace between the
           pieces, and an indented loop body would put a line break into the
           middle of every highlighted word. -->
      <p>{#each split(bubble.text, bubble.alerts, findAll(bubble.text, query)) as piece}{#if piece.alert || piece.hit}<mark class:hit={piece.hit} class:alert={piece.alert}>{piece.text}</mark>{:else}{piece.text}{/if}{/each}</p>
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

  /* Bright enough to be found from across a control room, and still readable
     while being read: the term is what is looked for, and the sentence around
     it is what is then read. Never a colour alone — the bubbles are already
     colour-coded by channel, and a ninth colour in the same row would compete
     with that. */
  mark {
    background: #f2b544;
    color: #12141a;
    border-radius: 0.15rem;
    padding: 0 0.1rem;
    font-weight: 600;
  }

  /* A search hit, which is not an alert and must not look like one (P9). The
     alert amber is the house's standing rule and is meant to be recognised
     across the room; this is the operator's own question of the moment, so it
     is the interface's own accent rather than a second warning colour. A piece
     that is both keeps the amber — the standing rule is the louder of the two
     and is what someone glancing over should still see. */
  mark.hit:not(.alert) {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: var(--text);
    font-weight: 600;
  }

  /* The line the ↑/↓ buttons are on. A ring and not a fill: the bubble already
     carries its channel's colour and its own highlights, and the one thing
     still to say is "this is the one you were brought to". */
  article.focused {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  /* --- The search bar (P9) ---------------------------------------------- */

  .search {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 1rem;
    border-bottom: 1px solid var(--border);
    /* Never scrolls away with the chat: the box is how the operator got to
       where they are, and it has to stay reachable from there. */
    flex: none;
  }

  .query {
    flex: 1;
    min-width: 0;
  }

  .query,
  .channel {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
  }

  .count {
    color: var(--text-dim);
    font-size: 0.8rem;
    /* Wide enough for "12 of 148 lines" so the buttons beside it do not move
       under the cursor as the count changes while typing. */
    min-width: 7rem;
    text-align: right;
  }

  .step {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.1rem 0.45rem;
    font: inherit;
    cursor: pointer;
  }

  .step:hover:not(:disabled) {
    color: var(--text);
  }

  .step:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .only {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-dim);
    font-size: 0.8rem;
    white-space: nowrap;
    cursor: pointer;
  }

  /* The arrival flash: bright for a moment, then the bubble it was. Long
     enough to be caught out of the corner of an eye, short enough that a busy
     group is not a list of permanently lit boxes — the flash says "this one is
     new", and a mark that stayed would say "this one is special".

     In the channel's own colour, so it also says *which* channel. */
  .arriving {
    animation: arriving 900ms ease-out;
  }

  @keyframes arriving {
    0% {
      background: color-mix(in oklab, var(--channel) 45%, var(--surface));
      border-color: var(--channel);
    }
    100% {
      background: var(--surface);
      border-color: var(--border);
    }
  }

  /* Some operators cannot be shown a flashing box at all, and the system says
     so. The line is still marked as the newest one for the same moment, just
     without the fade.

     Still an animation, and still the same length: `animation: none` would
     never end, and the end is what takes the bubble out of the arriving set —
     the mark would stay on every line for the rest of the session. */
  @media (prefers-reduced-motion: reduce) {
    .arriving {
      animation: arriving-quiet 900ms step-end;
    }

    @keyframes arriving-quiet {
      0% {
        border-color: var(--channel);
      }
      100% {
        border-color: var(--border);
      }
    }
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
