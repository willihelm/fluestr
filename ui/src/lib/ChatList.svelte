<script lang="ts">
  import { tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { TranscriptEvent } from "./bridge";
  import type { ClipKey, SpeechActivity } from "./bridge";
  import { audible, placeholders, toBubbles, type Bubble } from "./bubbles";
  import { replay, toggle, watchReplay } from "./replay.svelte";
  import { speech, watchSpeech } from "./speech.svelte";
  import { findAll, matches, split } from "./search";
  import {
    arrange,
    flip,
    fromNewest,
    fromOldest,
    label as directionLabel,
    loadOverride,
    newestEdge,
    saveOverride,
    type SortDirection,
  } from "./sort";
  import { currentGroup } from "./windows";

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
    /// Which end of the chat new lines arrive at, as the app's configuration
    /// has it (P7) — the default every view is born with and reads until it is
    /// turned over. Handed down rather than read from storage here: it is part
    /// of the configuration, and the component that renders the chat is not
    /// the one that talks to the backend.
    direction: SortDirection;
    /// Whether a session is capturing right now.
    ///
    /// Only the empty chat cares, and it cares a lot: "nothing transcribed
    /// yet" over a session that was never started reads as an app that is
    /// listening and hearing nothing, which is the same picture a dead
    /// interface makes. Required rather than defaulted — a view that renders
    /// the chat knows this, and guessing it wrong is exactly the confusion
    /// this prop exists to remove.
    running: boolean;
    /// How to start one from this window, for that empty state. `null` where
    /// there is nothing to press — a detached group window has no Start of its
    /// own, and naming a key that does nothing there is worse than saying
    /// where the button is.
    startHint?: string | null;
    /// The stream ids this view shows, or `null` for every one. `messages`
    /// already says this for the lines; the "speaking…" placeholders (P23)
    /// come before any line and need to be told.
    streams?: Set<number> | null;
    /// A line to bring on screen, asked for from a ticker (P24): any one
    /// segment of it, and a count that changes with every request, so that
    /// asking for the same line twice moves the chat twice.
    reveal?: { segment: ClipKey; nonce: number } | null;
  }

  let {
    messages,
    colors,
    starts,
    direction,
    running,
    startHint = null,
    streams = null,
    reveal: revealing = null,
  }: Props = $props();

  // Hearing a bubble again (P22). Every view follows the same horizon, and
  // plays its own replays — see `replay.svelte.ts`.
  watchReplay();

  // Who is talking before there is text (P23) — see `speech.svelte.ts`.
  watchSpeech();

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
  /// Which hit the ↑/↓ buttons are on, as a position in `hits`.
  let focus = $state(0);

  let searching = $derived(query.trim() !== "");

  // Consecutive segments of the same channel are one bubble; see
  // [`toBubbles`] for where a bubble ends.
  let bubbles = $derived(toBubbles(messages));

  /// The chat as this view has narrowed it: under the toggle, only the lines
  /// that match. Which channels are in it at all is the window's own group
  /// selection and nothing this view chooses — there is no picker to widen it
  /// past the group the operator opened.
  ///
  /// The window, the "N earlier" button and the follow-to-the-bottom effect
  /// all work on this rather than on `bubbles`, so a filtered chat scrolls and
  /// pages like any other — there is no second reading mode to maintain.
  let listed = $derived(
    bubbles.filter((b) => !onlyMatches || !searching || matches(b.text, query)),
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

  // --- Reading direction (P7) ----------------------------------------------
  //
  // The app's default, which this view may override, like the search above it
  // is this view's own: the desk reads the chat downwards while a second
  // screen may want the newest line under the eye at the top, and both are
  // views of the same chat. Turning one over is done here, on the window in
  // front of the operator; the default is set once, on the Settings page.
  //
  // The override is stored per view rather than held in the component, because
  // `App.svelte` unmounts the chat whenever Settings is shown — and an
  // operator who flips over to check a level must come back to the chat they
  // left. See `sort.ts` for why it is not in `config.json`.

  /// Which view this window is, for the storage key. Read once: a window does
  /// not become another view.
  const view = currentGroup();

  /// This view's own answer, or `null` while it follows the app's default —
  /// and the two are different states, because a default that changes moves a
  /// view that has not been turned and leaves one that has.
  let override = $state<SortDirection | null>(loadOverride(view));
  let reading = $derived(override ?? direction);
  let newestTop = $derived(reading === "newest-top");

  let limit = $state(PAGE);
  /// The page of the chat that is in the DOM: always the newest `limit`
  /// bubbles, whichever end they are then read from. Which lines are in the
  /// window is not a question of which way round they are shown.
  let page = $derived(listed.slice(Math.max(0, listed.length - limit)));
  let shown = $derived(arrange(page, reading));
  let hidden = $derived(listed.length - page.length);

  // --- "Speaking…" (P23) ---------------------------------------------------
  //
  // Placeholders are not bubbles and never pass through `toBubbles`: the rules
  // never see them, the search never matches them, and nothing about them is
  // kept. They sit at the newest edge, or as a line on the last bubble when
  // the words on their way will join it — see `placeholders`.

  let active = $derived(
    Object.values(speech.active)
      .map((entry) => entry.activity)
      .filter((a) => streams === null || streams.has(a.streamId)),
  );
  let waiting = $derived(placeholders(bubbles, active));
  /// The bubble carrying the indicator, when it is on the page. When it is
  /// not — scrolled out of the window, or hidden by "Only matches" — the
  /// channel gets a box of its own, since an indicator nobody can see is no
  /// indicator.
  let carrier = $derived(
    waiting.onLast && page.at(-1)?.key === bubbles.at(-1)?.key ? waiting.onLast : null,
  );
  let boxes = $derived(
    arrange(
      carrier === null && waiting.onLast
        ? [waiting.onLast, ...waiting.standalone]
        : waiting.standalone,
      reading,
    ),
  );
  let carrierKey = $derived(carrier ? (bubbles.at(-1)?.key ?? null) : null);

  const waitingText = (a: SpeechActivity) => (a.speaking ? "speaking" : "transcribing");
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
  // Follow behaviour: stay pinned to the end the newest line is at, but not
  // while the user is reading away from it. Which end that is comes from the
  // direction; everything below is written in terms of "the newest edge" so
  // that there is one follow behaviour rather than two. Still to come in P7: a
  // proper "N new messages" button.
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
    // Only when the history is above the viewport. With the newest line at
    // the top the page is appended *below* what is on screen, which moves
    // nothing — and
    // the correction would then scroll the operator away from the line they
    // were reading rather than hold them on it.
    if (container && !newestTop) container.scrollTop += container.scrollHeight - before;
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

  /// The bubble a ticker sent this view to (P24), marked for as long as its
  /// animation runs — how the operator's eye finds it in a chat that just
  /// moved under it.
  let revealed = $state<string | null>(null);

  // Brings a ticker's line on screen: the window grown far enough to hold it,
  // as `reveal` does for a hit, and the view stopped from chasing the newest
  // edge, because this is reading back.
  //
  // A search that hides the line is cleared first. The ticker was clicked to
  // see *that* line, and a chat that answers "nothing here" to it would be a
  // filter the operator set an hour ago deciding for them now.
  $effect(() => {
    const request = revealing;
    if (request === null) return;
    untrack(() => void bringOn(request.segment));
  });

  const bringOn = async (segment: ClipKey) => {
    const holds = (b: Bubble) =>
      b.segments.some(
        (s) =>
          s.sessionId === segment.sessionId &&
          s.streamId === segment.streamId &&
          s.seq === segment.seq,
      );
    let index = listed.findIndex(holds);
    if (index === -1 && bubbles.some(holds)) {
      query = "";
      onlyMatches = false;
      await tick();
      index = listed.findIndex(holds);
    }
    if (index === -1 || !container) return;

    following = false;
    const needed = listed.length - index;
    if (needed > limit) limit = Math.ceil(needed / PAGE) * PAGE;
    const key = listed[index].key;
    revealed = null;
    await tick();
    revealed = key;
    container
      ?.querySelector(`[data-key="${CSS.escape(key)}"]`)
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
    following = fromNewest(container, reading) < 80;
    // Near the history end of the window and there is more behind it: reach
    // for the next page before the operator hits the end of what is rendered,
    // so the history reads as one list rather than as pages. Which end that is
    // mirrors with the direction — the top with the newest line at the
    // bottom, the bottom with the newest line at the top.
    //
    // Only while reading back. At the newest edge the follow effect trims the
    // window again, and a chat shorter than the viewport is at both edges at
    // once — growing and trimming each other in a loop.
    if (!following && fromOldest(container, reading) < 400) void showMore();
  };

  $effect(() => {
    // Touch the message count, not the bubble count: a segment appended to
    // the last bubble leaves the number of bubbles unchanged while the box
    // still grows, and the view has to follow that too.
    messages.length;
    // And a placeholder appearing: it is the newest thing in the view.
    boxes.length;
    if (following && container) {
      // Back at the newest edge, the pages that were opened to read backwards
      // are no longer being looked at. Dropping them keeps a session that ran
      // for hours from carrying every node it ever rendered — and it is
      // invisible, because everything dropped is off the far end of the
      // viewport.
      limit = PAGE;
      container.scrollTop = newestEdge(container, reading);
    }
  });

  // The direction can also change from outside this view: the app's default is
  // on the Settings page, and a window that has not been turned over follows
  // it while it is open. A view that was watching the newest line keeps
  // watching it, at whichever edge that now is — without this, the chat would
  // silently flip and leave the operator looking at the oldest bubble of the
  // page. A view that was reading back is left where it is: the lines around
  // it are the same lines, and moving somebody who is reading is worse than
  // letting them see the list turn over.
  $effect(() => {
    reading;
    untrack(() => {
      if (following && container) container.scrollTop = newestEdge(container, reading);
    });
  });

  /// Turns this window round (P7).
  ///
  /// The same bubbles in the inverse visual order, and the operator kept where
  /// they were reading: the line in the middle of the viewport is found before
  /// the flip and brought back to the middle after it. Without that, turning
  /// the chat over while reading an hour back would land on whichever lines
  /// happen to be at that scroll offset from the other end, which is nowhere.
  ///
  /// A window that was following the chat simply follows it at the other edge
  /// — there is no position to keep, and the newest line is the one being
  /// watched.
  ///
  /// The bar offers the two directions and not a third "follow the app"
  /// state: a window is read the way it looks, and a control that could be
  /// pressed without the chat changing is a control that reads as broken. A
  /// view that has been turned therefore keeps its own answer until it is
  /// turned back by hand — which is one press, in the same place.
  const turn = async (to: SortDirection) => {
    const anchor = following ? null : middleKey();

    // Stored for this view. Kept across the trips to Settings that unmount
    // the chat, and kept across a restart: a second screen that is read upside
    // down is read upside down every evening.
    override = to;
    saveOverride(view, to);
    await tick();
    if (!container) return;

    if (anchor === null) {
      container.scrollTop = newestEdge(container, reading);
      return;
    }
    container
      .querySelector(`[data-key="${CSS.escape(anchor)}"]`)
      ?.scrollIntoView({ block: "center" });
  };

  /// The key of the bubble across the middle of the viewport, or `null` when
  /// there is none — an empty view, or one whose only bubble is taller than
  /// the window.
  ///
  /// The middle rather than the first line on screen: a bubble is several
  /// lines tall, and the one an operator is reading is the one their eye is
  /// on, not the one clipped by the top edge.
  const middleKey = (): string | null => {
    if (!container) return null;
    const box = container.getBoundingClientRect();
    const middle = box.top + box.height / 2;

    for (const node of container.querySelectorAll<HTMLElement>("[data-key]")) {
      const rect = node.getBoundingClientRect();
      if (rect.bottom >= middle && rect.top <= middle) return node.dataset.key ?? null;
    }
    return null;
  };

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
  <!-- The reading direction of the list below (P7), at the head of the bar and
       never anywhere else in it: it is the first thing said about what follows,
       and the search controls that appear as the operator types must not push
       it out from under the cursor.

       One press, because turning the chat over is done during a show and there
       are only two directions — a menu would be three clicks for a control with
       two states. Labelled with the direction it is *in* rather than the one it
       would switch to: it is a statement about the chat below it, the way
       "Only matches" is, and the arrow is what says it can be turned. -->
  <button
    class="direction"
    onclick={() => turn(flip(reading))}
    aria-pressed={newestTop}
    title={newestTop
      ? "Newest at the top — click to read the chat downwards"
      : "Newest at the bottom — click to bring the newest line to the top"}
  >
    <span class="arrow" aria-hidden="true">{newestTop ? "↑" : "↓"}</span>
    {directionLabel(reading)}
  </button>

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
  {#if messages.length === 0 && boxes.length === 0}
    <p class="empty">
      {#if running}
        Nothing transcribed yet. Speak into one of the channels.
      {:else}
        {startHint ?? "No session is running — start one in the main window."}
      {/if}
    </p>
  {:else if listed.length === 0}
    <!-- The chat is not empty, the view is — which is a different thing to be
         told, because it has a cause the operator can undo. -->
    <p class="empty">
      Nothing here matches. Clear the search or widen it to see the chat again.
    </p>
  {/if}

  <!-- At the history end of the list, which mirrors with the direction: above
       the oldest bubble reading newest-last, below it reading newest-first.
       One snippet in two places rather than a CSS `order`, because where it is
       in the document is also where a keyboard and a screen reader meet it,
       and "show more" belongs beside the lines it would add. -->
  {#snippet earlier()}
    {#if hidden > 0}
      <button class="earlier" onclick={showMore}>
        {hidden} earlier {hidden === 1 ? "bubble" : "bubbles"} — show more
      </button>
    {/if}
  {/snippet}

  {#if !newestTop}{@render earlier()}{/if}

  <!-- A channel that is talking before its text has arrived (P23). In the
       channel's colour and under its label, where its bubble will land;
       the transcript replaces it. No key, no time, no latency — there is
       nothing yet to time, and nothing to replay or search. -->
  {#snippet placeholder(a: SpeechActivity)}
    <article class="placeholder" style="--channel: {colorOf(a.streamId)}">
      <header>
        <span class="label">{a.label || "Stream " + a.streamId}</span>
      </header>
      {@render indicator(a)}
    </article>
  {/snippet}

  {#snippet indicator(a: SpeechActivity)}
    <p class="indicator" aria-label="{a.label || 'Stream ' + a.streamId} is {waitingText(a)}">
      {waitingText(a)}<span class="dots" aria-hidden="true"><span>.</span><span>.</span><span>.</span></span>
    </p>
  {/snippet}

  {#if newestTop}
    {#each boxes as a (a.streamId)}{@render placeholder(a)}{/each}
  {/if}

  <!-- Keyed on the identity of the segment that opened the bubble. On all
       three parts, not on `streamId + seq`: those repeat across sessions, and
       a duplicate key throws and stops the list rendering entirely — which is
       what "nothing appears after Stop and Start" looked like. -->
  {#each shown as bubble (bubble.key)}
    <article
      class:arriving={arriving.has(bubble.key)}
      class:focused={bubble.key === focused}
      class:revealed={bubble.key === revealed}
      data-focused={bubble.key === focused}
      onanimationend={() => {
        arriving.delete(bubble.key);
        if (revealed === bubble.key) revealed = null;
      }}
      data-key={bubble.key}
      style="--channel: {colorOf(bubble.streamId)}"
    >
      <header>
        <span class="label">{bubble.label || "Stream " + bubble.streamId}</span>
        <span class="time" title={clockTitle(bubble)}>{clock(bubble)}</span>
        <span class="latency" title={latencyTitle(bubble)}>
          {(bubble.latencyMs / 1000).toFixed(1)} s
        </span>
        <!-- Only while the audio is held: a button that fails when pressed is
             worse than none, and the oldest audio goes first (P22). -->
        {#if audible(bubble, replay.horizon)}
          <button
            class="replay"
            class:playing={replay.playing === bubble.key}
            onclick={() => toggle(bubble.key, bubble.segments)}
            title={replay.playing === bubble.key ? "Stop" : "Listen to what was said"}
            aria-label={replay.playing === bubble.key ? "Stop the replay" : "Listen to this message"}
          >
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              {#if replay.playing === bubble.key}
                <rect x="2" y="2" width="8" height="8" fill="currentColor" />
              {:else}
                <path d="M3 1.5v9l7.5-4.5z" fill="currentColor" />
              {/if}
            </svg>
          </button>
        {/if}
      </header>
      <!-- The alert terms, marked where the backend found them (P17). Written
           out piece by piece rather than through `{@html}`: a transcript is
           text from outside, and the one thing this view must never do is let
           a spoken sentence become markup.

           On one line on purpose — Svelte keeps the whitespace between the
           pieces, and an indented loop body would put a line break into the
           middle of every highlighted word. -->
      <p>{#each split(bubble.text, bubble.alerts, findAll(bubble.text, query)) as piece}{#if piece.alert || piece.hit}<mark class:hit={piece.hit} class:alert={piece.alert}>{piece.text}</mark>{:else}{piece.text}{/if}{/each}</p>
      {#if replay.error?.key === bubble.key}
        <p class="replay-error">Could not play: {replay.error.message}</p>
      {/if}
      {#if carrier && bubble.key === carrierKey}{@render indicator(carrier)}{/if}
    </article>
  {/each}

  {#if !newestTop}
    {#each boxes as a (a.streamId)}{@render placeholder(a)}{/each}
  {/if}

  {#if newestTop}{@render earlier()}{/if}
</div>

<!-- Stays at the foot of the window whichever way round the chat is read.
     What it does follows the direction — it lands on the newest line, at the
     top or at the bottom — but where it *is* must not, because it is the one
     control an operator reaches for without looking, and it is absolutely
     placed over the header bars at the other end of the window. -->
{#if !following}
  <button
    class="jump"
    onclick={() => {
      following = true;
      if (container) container.scrollTop = newestEdge(container, reading);
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
    /* The list scrolls under the phone's home indicator and out from behind
       the rounded corners; the bubbles themselves stop clear of both. */
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
    padding-bottom: max(1rem, var(--safe-bottom));
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

  /* Small and in the header's own dim colour: it is there when the text is
     doubted, and must not compete with the text when it is not. */
  .replay {
    align-self: center;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.35rem;
    height: 1.35rem;
    padding: 0;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 50%;
    cursor: pointer;
  }

  .replay:hover {
    color: var(--text);
  }

  .replay.playing {
    color: var(--channel);
    border-color: var(--channel);
  }

  .replay-error {
    margin-top: 0.2rem;
    font-size: 0.75rem;
    color: #ff6b6b;
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

  /* The line a ticker sent this view to (P24): ringed in its channel's colour
     until the eye has had time to land on it, then gone — it is an answer to
     "where", not a state. */
  article.revealed {
    animation: revealed 1.8s ease-out;
  }

  @keyframes revealed {
    0%,
    40% {
      outline: 2px solid var(--channel);
      outline-offset: 2px;
    }
    100% {
      outline: 2px solid transparent;
      outline-offset: 2px;
    }
  }

  /* --- The search bar (P9) ---------------------------------------------- */

  .search {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 1rem;
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
    border-bottom: 1px solid var(--border);
    /* Never scrolls away with the chat: the box is how the operator got to
       where they are, and it has to stay reachable from there. */
    flex: none;
    /* The bar grows a second line rather than squeezing the search box down
       to nothing: the row is six controls wide once a search is running, and
       a phone has room for about three of them. Which ones end up on the
       second line follows the order they are written in, which is also the
       order they are used in — direction, then the query, then what to do
       with its matches. */
    flex-wrap: wrap;
  }

  .query {
    flex: 1;
    min-width: 0;
  }

  .query {
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

  .jump {
    position: absolute;
    /* Above the home indicator, not under it. */
    bottom: calc(1.5rem + var(--safe-bottom));
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

  /* --- A window the width of a hand --------------------------------------

     Same breakpoint as everywhere else; see `layout.ts`. The search bar is
     the part of this view that was drawn for a row of controls across a desk,
     and what it needs on a phone is for the box to have the first line to
     itself. */
  @media (max-width: 45rem) {
    .search {
      padding: 0.45rem 0.7rem;
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
      row-gap: 0.35rem;
    }

    /* The whole of the line it is on, after the direction button beside it:
       a search field shared with two step buttons is three half-width
       controls and no room to read what was typed. */
    .query {
      flex: 1 0 8rem;
    }

    /* The count no longer holds a fixed width: on the desk it is stopping the
       buttons beside it from shifting, and here it is the thing that decides
       whether those buttons fit on the line at all. It is read after the
       typing stops either way. */
    .count {
      min-width: 0;
      text-align: left;
    }

    .chat {
      padding: 0.7rem;
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
      padding-bottom: max(0.7rem, var(--safe-bottom));
    }

    /* A bubble is as wide as the phone. The 48 rem cap exists so a line of
       transcript is not a paragraph the width of a 27" display; at this size
       there is nothing to cap. */
    article {
      max-width: none;
    }
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

  /* Dim and quiet until it is on, like the group window's pin: a control that
     looks the same in both of its states is a control nobody trusts. */
  .direction {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.5rem;
    font: inherit;
    font-size: 0.8rem;
    white-space: nowrap;
    cursor: pointer;
  }

  .direction:hover {
    color: var(--text);
  }

  .direction[aria-pressed="true"] {
    color: var(--accent);
    background: var(--bg);
  }

  /* Tabular so the label does not shift sideways as the arrow turns over. */
  .arrow {
    font-variant-numeric: tabular-nums;
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

  /* --- "Speaking…" (P23) ------------------------------------------------ */

  /* A bubble that has not arrived yet: the channel's edge and label like any
     other, on a dashed border and no fill, so it never reads as a line of
     text that is somehow empty. */
  .placeholder {
    background: transparent;
    border-style: dashed;
    border-left-style: solid;
  }

  .indicator {
    color: var(--channel);
    font-size: 0.85rem;
    font-style: italic;
  }

  article:not(.placeholder) .indicator {
    margin-top: 0.15rem;
  }

  .dots span {
    animation: dot 1.2s infinite ease-in-out;
    opacity: 0.2;
  }

  .dots span:nth-child(2) {
    animation-delay: 0.2s;
  }

  .dots span:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes dot {
    0%,
    80%,
    100% {
      opacity: 0.2;
    }
    40% {
      opacity: 1;
    }
  }

  /* Some operators cannot be shown a flashing box at all, and the system says
     so. The line is still marked as the newest one for the same moment, just
     without the fade.

     Still an animation, and still the same length: `animation: none` would
     never end, and the end is what takes the bubble out of the arriving set —
     the mark would stay on every line for the rest of the session. */
  @media (prefers-reduced-motion: reduce) {
    /* The words say it; the dots only have to be there. */
    .dots span {
      animation: none;
      opacity: 1;
    }

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

</style>
