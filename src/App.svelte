<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';


  let state = $state({
    hn_url: '', api_key: '', requirements: '', pdf_path: null,
    cache_key: null, processing: { enabled: false, total: 0, done: 0 },
    comments: [], evaluations: {}, sort_column: 'CreatedAt', descending: false
  });
  let flags = $state({});
  let rows = $state({});
  let eval_count = 0;
  async function refresh() {
      state = await invoke('get_state');
      rows = (await invoke('get_rows')).map(c => c.comment);
      for (c of rows) {
        await getFlags(c.id);
      }
      eval_count  = state.comments.filter((c) => state.evaluations[c.id]).length;
  };
  onMount(() => {
    const unlisten = listen('refresh-rows', async (event) => {

      console.log(event);
    
    });
      return () => unlisten.then(f => f());
  });
  async function getFlags(commentId) {
      if (flags[commentId] !== undefined) return;
      flags[commentId] = await invoke('comment_flags', { commentId });
  }
  // Poll for state updates (processing progress, new comments, evaluations)
  onMount(() => {
    const interval = setInterval(async () => {
      await refresh();
    }, 1000);
    return () => clearInterval(interval);
  });
  let saveTimer;
  function debouncedSave() {
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => invoke('save_state'), 10000);
  }
  debouncedSave();

  async function selectPdf() {
    const path = await invoke('select_pdf');
    if (path) state.pdf_path = path;
  }

  async function setSort(column) {
    await invoke('set_sort', { column });
    state = await invoke('get_state');
  }

  function sortedComments2() {
    const sorted = [...state.comments].sort((a, b) => {
      let res = 0;
      if (state.sort_column === 'Score') {
        const sa = state.evaluations[a.id]?.score ?? 0;
        const sb = state.evaluations[b.id]?.score ?? 0;
        res = sa - sb;
      } else if (state.sort_column === 'Id') {
        res = a.id - b.id;
      } else {
        res = new Date(a.created_at) - new Date(b.created_at);
      }
      return state.descending ? -res : res;
    });
    console.log(sorted);
    return sorted;
  }
</script>

<main>
  <h1>HN Comment Evaluator</h1>

  <div class="controls">
    <div class="inputs">
      <input
        placeholder="HN URL"
        bind:value={state.hn_url}
        onchange={() => updateField('hn_url', state.hn_url)}
      />
      <input
        placeholder="Gemini API Key"
        type="password"
        bind:value={state.api_key}
        onchange={() => updateField('api_key', state.api_key)}
      />
      <div class="status">
        {#if state.pdf_path}<span>Selected: {state.pdf_path}</span>{/if}
        <span>Cache key: {state.eval_cache ? 'OK' : 'None'}</span>
        {#if state.processing.enabled}
          <span>Processing: {state.processing.done}/{state.processing.total}</span>
        {:else}
          <span>Processing disabled</span>
        {/if}
      </div>

      <div class="status">
        <span>Count: {eval_count}/{state.comments.length}</span>
      </div>
      
      <div class="buttons">
        <button onclick={selectPdf}>Select PDF</button>
        <button onclick={() => invoke('get_evaluation_cache')}>Get Ev Cache</button>
        <button onclick={() => invoke('process_comments', { force: false })}>Process Comments</button>
        <button onclick={() => invoke('process_comments', { force: true })}>Force Process</button>
        <button onclick={() => invoke('batch_process')}>Batch Process</button>
        <button onclick={() => invoke('nuke_evaluations')}>Nuke Evaluations</button>
      </div>
    </div>
    <textarea
      placeholder="Requirements"
      rows="5"
      bind:value={state.requirements}
      onchange={() => updateField('requirements', state.requirements)}
    ></textarea>
  </div>

  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th style="width: 11em;" onclick={() => setSort('CreatedAt')} class="sortable">CreatedAt</th>
          <th onclick={() => setSort('Id')} class="sortable">Comment</th>
          <th>Evaluation</th>
          <th>Tech Alignment</th>
          <th>Comp Alignment</th>
          <th style="width: 5%;" onclick={() => setSort('Score')} class="sortable">Score</th>
          <th style="width: 10%;"></th>
        </tr>
      </thead>
      <tbody>
        {#each rows as comment (comment.id)}
          {@const eval_ = state.evaluations[comment.id]}
          {#await getFlags(comment.id) then _}
            <tr class:seen={flags[comment.id].seen} class:in-progress={flags[comment.id].in_progress}>
            <td class="col-date">{comment.created_at}</td>
            <td class="col-comment">
              <a onclick={() => invoke("open_url", {url: "https://news.ycombinator.com/item?id="+comment.id})} href="https://news.ycombinator.com/item?id={comment.id}" target="_blank">
                #{comment.id}
              </a>
              <div class="scrollable">{comment.text ?? ''}</div>
            </td>
            <td>
              {#if eval_}<div class="scrollable">{eval_.evaluation}</div>
              {:else}-{/if}
            </td>
            <td>
              {#if eval_}<div class="scrollable">{eval_.technology_alignment}</div>
              {:else}-{/if}
            </td>
            <td>
              {#if eval_}<div class="scrollable">{eval_.compensation_alignment}</div>
              {:else}-{/if}
            </td>
            <td class="col-score">{eval_?.score ?? '-'}</td>
            <td class="control-buttons">
              <button
                disabled={!state.cache_key}
                onclick={() => invoke('evaluate_comment_cmd', { commentId: comment.id })}
              >Evaluate</button>
              {#if flags[comment.id]?.hidden}
                <button 
                  onclick={() => invoke('comment_flag_hide', { commentId: comment.id, toggle: false})}
                  >Unhide</button>
              {:else}
                <button 
                  onclick={() => invoke('comment_flag_hide', { commentId: comment.id, toggle: true})}
                  >Hide</button>
              {/if}
              {#if flags[comment.id]?.seen}
                <button 
                  onclick={() => invoke('comment_flag_seen', { commentId: comment.id, toggle: false})}
                  >Not seen</button>
              {:else}
                <button 
                  onclick={() => invoke('comment_flag_seen', { commentId: comment.id, toggle: true})}
                  >Seen</button>
              {/if}
              {#if flags[comment.id]?.in_progress}
                <button 
                  onclick={() => invoke('comment_flag_in_progress', { commentId: comment.id, toggle: false})}
                  >Not in progress</button>
              {:else}
                <button 
                  onclick={() => invoke('comment_flag_in_progress', { commentId: comment.id, toggle: true})}
                  >Set in progress</button>
              {/if}
            </td>
          </tr>
          {/await}
        {/each}
      </tbody>
    </table>
  </div>
</main>

<style>
  main { display: flex; flex-direction: column; height: 100vh; padding: 1rem; gap: 1rem; font-family: sans-serif; background: #2d353b; color: #d3c6aa; }
  h1 { text-align: center; margin: 0; }
  .controls { display: flex; gap: 1rem; }
  .inputs { display: flex; flex-direction: column; gap: 0.5rem; min-width: 320px; }
  .buttons { display: flex; flex-wrap: wrap; gap: 0.4rem; }
  .status { display: flex; flex-direction: column; font-size: 0.85rem; gap: 0.2rem; }
  .control-buttons button { width: 100%; margin-bottom: 10pt; }
  input, textarea { background: #333c43; border: 1px solid #495157; color: #d3c6aa; padding: 0.4rem; border-radius: 3px; }
  textarea { flex: 1; resize: vertical; }
  button { background: #3d4841; border: 1px solid #495157; color: #d3c6aa; padding: 0.3rem 0.6rem; border-radius: 3px; cursor: pointer; }
  button:hover:not(:disabled) { background: #a7c080; color: #2d353b; }
  button:disabled { opacity: 0.4; cursor: default; }
  .table-wrap { flex: 1; overflow-y: auto; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  thead { position: sticky; top: 0; background: #2d353b; }
  th { padding: 0.5rem; border-bottom: 1px solid #495157; text-align: left; }
  th.sortable { cursor: pointer; }
  th.sortable:hover { color: #a7c080; }
  td { padding: 0.5rem; border-bottom: 1px solid #3d4841; vertical-align: top; }
  tr:nth-child(even) { background: #333c43; }
  .scrollable { max-height: 200px; overflow-y: auto; font-size: 0.65rem; text-align: justify; line-height: 1.4; }
  .col-date { width: 10%; }
  .in-progress { color: rgb(100,255,100); }
  .seen { opacity: 0.2 }
  .col-comment { width: 37%; }
  .col-score { width: 4%; text-align: center; }
  a { color: #a7c080; font-size: 0.8rem; }
  td a { font-size: 0.4rem; }
  td { font-size: 0.65rem; }
  :global(body) { background: #2d353b; margin: 0; }
  input:focus, textarea:focus { outline: none; border-color: #a7c080; }
</style>
