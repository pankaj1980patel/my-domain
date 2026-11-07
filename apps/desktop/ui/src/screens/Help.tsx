// Help: short topics on what each feature does, what it needs and what it
// can't do, from docs/help. A search box (every word, in the title, keywords
// or text), the topic list, and the chosen topic.

import { Icon, IconChip, type IconName, type Tone } from "../components/Icon";
import { createEffect, createMemo, createSignal, For, Match, on, Show, Switch } from "solid-js";
import { helpTarget, type Block, type Inline } from "../help/markdown";
import { searchTopics } from "../help/search";
import { topicById, TOPICS } from "../help/topics";
import { t } from "../i18n";
import type { Key } from "../i18n";

function Inlines(props: { nodes: Inline[]; onTopic: (id: string) => void }) {
  return (
    <For each={props.nodes}>
      {(n) => (
        <Switch>
          <Match when={n.kind === "text" && n}>{(x) => x().text}</Match>
          <Match when={n.kind === "code" && n}>{(x) => <code>{x().text}</code>}</Match>
          <Match when={n.kind === "bold" && n}>
            {(x) => (
              <strong>
                <Inlines nodes={x().children} onTopic={props.onTopic} />
              </strong>
            )}
          </Match>
          <Match when={n.kind === "italic" && n}>
            {(x) => (
              <em>
                <Inlines nodes={x().children} onTopic={props.onTopic} />
              </em>
            )}
          </Match>
          <Match when={n.kind === "link" && n}>
            {(x) => {
              const href = helpTarget(x().href);
              // A topic only the other app lists stays plain text here.
              const target = href && topicById(href) ? href : null;
              // Web links aren't opened from the app's window: shown as text.
              return target ? (
                <a
                  href={`#help-${target}`}
                  class="help-xref"
                  onClick={(e) => {
                    e.preventDefault();
                    props.onTopic(target);
                  }}
                >
                  <Inlines nodes={x().children} onTopic={props.onTopic} />
                </a>
              ) : (
                <span>
                  <Inlines nodes={x().children} onTopic={props.onTopic} />
                  <Show when={!href}>
                    {" "}
                    <span class="muted">({x().href})</span>
                  </Show>
                </span>
              );
            }}
          </Match>
        </Switch>
      )}
    </For>
  );
}

function Blocks(props: { blocks: Block[]; onTopic: (id: string) => void }) {
  return (
    <For each={props.blocks}>
      {(b) => (
        <Switch>
          <Match when={b.kind === "heading" && b}>
            {(h) => (
              <Show
                when={h().level <= 1}
                fallback={
                  <Show
                    when={h().level === 2}
                    fallback={
                      <h4>
                        <Inlines nodes={h().inlines} onTopic={props.onTopic} />
                      </h4>
                    }
                  >
                    <h3>
                      <Inlines nodes={h().inlines} onTopic={props.onTopic} />
                    </h3>
                  </Show>
                }
              >
                <h2>
                  <Inlines nodes={h().inlines} onTopic={props.onTopic} />
                </h2>
              </Show>
            )}
          </Match>
          <Match when={b.kind === "para" && b}>
            {(p) => (
              <p>
                <Inlines nodes={p().inlines} onTopic={props.onTopic} />
              </p>
            )}
          </Match>
          <Match when={b.kind === "list" && b}>
            {(l) => {
              const items = (
                <For each={l().items}>
                  {(item) => (
                    <li>
                      <Inlines nodes={item} onTopic={props.onTopic} />
                    </li>
                  )}
                </For>
              );
              return l().ordered ? <ol>{items}</ol> : <ul>{items}</ul>;
            }}
          </Match>
        </Switch>
      )}
    </For>
  );
}

const HELP_CARDS: { topic: string; icon: IconName; tone: Tone; title: Key; body: Key }[] = [
  { topic: "getting-started", icon: "home", tone: "cyan", title: "help.cardStart", body: "help.cardStartBody" },
  { topic: "messages-clipboard", icon: "quick", tone: "violet", title: "help.cardFeatures", body: "help.cardFeaturesBody" },
  { topic: "privacy", icon: "shield", tone: "green", title: "help.cardPrivacy", body: "help.cardPrivacyBody" },
];

export default function Help(props: { topic?: string; onTopic: (id: string) => void }) {
  const [query, setQuery] = createSignal("");
  const shown = createMemo(() => searchTopics(TOPICS, query()));
  // The chosen topic, unless a search leaves it out: then the best match.
  const current = () => {
    const chosen = topicById(props.topic);
    const searching = !!query().trim();
    if (chosen && (!searching || shown().includes(chosen))) return chosen;
    return shown()[0];
  };
  let article: HTMLElement | undefined;

  // A new topic starts at its top.
  createEffect(
    on(
      () => current()?.id,
      () => article?.scrollIntoView({ block: "nearest" }),
      { defer: true },
    ),
  );

  return (
    <section class="page page-fit help-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="help" size={26} tone="cyan" />
        </span>
        <div class="grow">
          <h1>{t("help.title")}</h1>
          <p class="muted">{t("help.subtitle")}</p>
        </div>
      </header>

      <input
        class="help-search"
        type="search"
        placeholder={t("help.search")}
        aria-label={t("help.searchLabel")}
        autocomplete="off"
        spellcheck={false}
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
      />

      {/* Where to start: three cards, each opening its topic. */}
      <div class="help-cards">
        <For each={HELP_CARDS}>
          {(c) => (
            <button class="tile help-card" aria-current={props.topic === c.topic ? "true" : undefined} onClick={() => props.onTopic(c.topic)}>
              <IconChip name={c.icon} tone={c.tone} />
              <span class="grow">
                <strong>{t(c.title)}</strong>
                <span class="muted small">{t(c.body)}</span>
              </span>
            </button>
          )}
        </For>
      </div>

      <div class="help-layout">
        <nav class="help-topics" aria-label={t("help.topics")}>
          <Show when={shown().length} fallback={<p class="muted small">{t("help.none", { query: query().trim() })}</p>}>
            <ul>
              <For each={shown()}>
                {(topic) => (
                  <li>
                    <button
                      type="button"
                      class="help-topic"
                      classList={{ active: current()?.id === topic.id }}
                      aria-current={current()?.id === topic.id ? "page" : undefined}
                      onClick={() => props.onTopic(topic.id)}
                    >
                      {topic.title}
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </nav>

        <Show when={current()}>
          {(topic) => (
            <article class="tile help-article" ref={article} aria-labelledby="help-article-title">
              <h2 id="help-article-title" class="help-article-title">
                {topic().title}
              </h2>
              <div class="help-body">
                <Blocks blocks={topic().blocks} onTopic={props.onTopic} />
              </div>
            </article>
          )}
        </Show>
      </div>
    </section>
  );
}
