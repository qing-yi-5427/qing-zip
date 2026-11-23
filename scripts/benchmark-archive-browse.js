// The Python runner supplies the actual product functions and CLI JSON bytes.
const columns = ArchiveColumns.normalize(null);
const s = {selected: new Set(), listStyle: 'details', flatView: false, sort: 'name', reverse: false};
const viewport = document.querySelector('#viewport');
const body = viewport.querySelector('tbody');

function measure(operation) {
  const started = performance.now();
  const result = operation();
  return [result, Number((performance.now() - started).toFixed(3))];
}

function view(folder, query, scope, expectedCount) {
  Object.assign(s, {folder, search: query, searchScope: scope});
  const [list, lookupMs] = measure(() => {
    const list = visibleEntries();
    positionsFor(list);
    return list;
  });
  if (list.length !== expectedCount) {
    throw new Error(`Expected ${expectedCount} rows for ${scope}/${folder}/${query}, got ${list.length}`);
  }

  s.cursor = list[0]?.name;
  viewport.scrollTop = 0;
  const [window, mountMs] = measure(() => {
    if (list.length > windowThreshold) {
      return new ArchiveRowWindow(body, viewport, list, fileRow, () => 0, 6, JSON.stringify(columns));
    }
    body.innerHTML = list.map(fileRow).join('');
    body.getBoundingClientRect();
    return null;
  });
  window?.stop();
  const mounted = body.querySelectorAll('[data-row]');
  if (mounted.length > Math.min(64, expectedCount) || (expectedCount && !mounted.length)) {
    throw new Error(`Unexpected first window size: ${mounted.length}`);
  }
  if (query === 'target-needle' && mounted[0].dataset.row !== 'wide/target-needle.txt') {
    throw new Error('Wrong search match');
  }
  return {lookup_ms: lookupMs, mount_ms: mountMs, logical_rows: list.length, dom_rows: mounted.length};
}

function round(bytes, count) {
  const [decoded, decodeMs] = measure(() => QingArchiveListing.read(bytes));
  s.entries = decoded.entries;
  if (s.entries.length !== count) throw new Error('Wrong number of decoded entries');

  return {
    utf8_json_ms: decodeMs,
    root: view('', '', 'folder', count / 1000 + 1),
    wide: view('wide/', '', 'folder', count / 2),
    current_match: view('wide/', 'target-needle', 'folder', 1),
    archive_match: view('wide/', 'target-needle', 'archive', 1),
    archive_broad: view('wide/', 'member-', 'archive', count - 1),
    archive_no_match: view('wide/', 'missing-sample', 'archive', 0),
    js_heap_bytes: performance.memory?.usedJSHeapSize ?? null,
  };
}

const destination = document.querySelector('#result');
try {
  const bytes = Uint8Array.from(atob(sample.base64), character => character.charCodeAt(0));
  const runs = Array.from({length: sample.rounds + 1}, () => round(bytes, sample.count));
  destination.textContent = JSON.stringify({user_agent: navigator.userAgent, warmup: runs[0], samples: runs.slice(1)});
} catch (error) {
  destination.textContent = JSON.stringify({error: error.message});
}
