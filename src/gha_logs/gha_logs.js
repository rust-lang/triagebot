// Logic for triagebot GitHub Actions logs viewer

const logsEl = document.getElementById("logs");
const ansi_up = new AnsiUp();
ansi_up.use_classes = true;

let startingAnchorId = null;

// 1. Tranform the ANSI escape codes to HTML
var html = ansi_up.ansi_to_html(logs);

// 2. Remove UTF-8 useless BOM and Windows Carriage Return
html = html.replace(/^\uFEFF/gm, "");
html = html.replace(/\r\n/g, "\n");

// 3 Transform each log lines.
//  If it starts start with a timestamp, make two column and make the timestamp be a self-referencial anchor.
//  If it doesn't start with a timestamp, put everything in the second column.
const tsRegex = /^(?:(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z) )?(.*)/;
const lines = html.split('\n');

html = "";
for (let line of lines) {
    const [, ts, log] = line.match(tsRegex);
    if (ts !== undefined) {
        html += `<tr><td><a id="${ts}" href="#${ts}" class="timestamp" data-pseudo-content="${ts}"></a></td><td>${log}</td></tr>`;
    } else {
        html += `<tr><td></td><td>${log}</td></tr>`;
    }
}

// 4. Add a anchor around every "##[error]" string
let errorCounter = -1;
html = html.replace(/##\[error\]/g, () =>
    `<a id="error-${++errorCounter}" class="error-marker">##[error]</a>`
);

// 4.b Add a span around every "##[warning]" string
html = html.replace(/##\[warning\]/g, () =>
    `<span class="warning-marker">##[warning]</span>`
);

// pre-5. Polyfill the recently (2025) added `RegExp.escape` function.
//  Inspired by the former MDN section on escaping:
//  https://web.archive.org/web/20230806114646/https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Regular_expressions#escaping
const escapeRegExp = RegExp.escape || function(string) {
    return string.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
};

// 5. Add anchors around some paths
//  Detailed examples of what the regex does is at https://regex101.com/r/vCnx9Y/2
//
//  But simply speaking the regex tries to find absolute (with `/checkout` prefix) and
//  relative paths, the path must start with one of the repository level-2 directories.
//  We also try to retrieve the lines and cols if given (`<path>:line:col`).
//
//  Some examples of paths we want to find:
//   - src/tools/test-float-parse/src/traits.rs:173:11
//   - /checkout/compiler/rustc_macros
//   - /checkout/src/doc/rustdoc/src/advanced-features.md
//
//  Any other paths, in particular if prefixed by `./` or `obj/` should not taken.
const pathRegex = new RegExp(
    "(?<boundary_start>[^a-zA-Z0-9.\\/])"
    + "(?<inner>(?:[\\\/]?(?:checkout[\\\/])?(?<path>(?:"
        + tree_roots.map(p => escapeRegExp(p)).join("|")
        + ")(?:[\\\/][a-zA-Z0-9_$\\\-.\\\/]+)?))"
    + "(?::(?<line>[0-9]+):(?<col>[0-9]+))?)(?<boundary_end>[^a-zA-Z0-9.])",
    "g"
);
html = html.replace(pathRegex, (match, boundary_start, inner, path, line, col, boundary_end) => {
    const pos = (line !== undefined) ? `#L${line}` : "";
    return `${boundary_start}<a href="https://github.com/${owner}/${repo}/blob/${sha}/${path}${pos}" class="path-marker">${inner}</a>${boundary_end}`;
});

// 6. Add the html to the table
logsEl.innerHTML = html;

// 7. If no anchor is given, scroll to the last error
if (location.hash === "" && errorCounter >= 0) {
    const hasSmallViewport = window.innerWidth <= 750;
    document.getElementById(`error-${errorCounter}`).scrollIntoView({
        behavior: 'instant',
        block: 'end',
        inline: hasSmallViewport ? 'start' : 'center'
    });
}

// 8. If a anchor is given, highlight and scroll to the selection
if (location.hash !== "") {
    const match = window.location.hash
        .match(/L?(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z)(?:-L(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z))?/);

    if (match) {
        const [startId, endId] = [match[1], match[2] || match[1]].map(decodeURIComponent);
        const startRow = logsEl.querySelector(`a[id="${startId}"]`)?.closest('tr');

        if (startRow) {
            startingAnchorId = startId;
            highlightTimestampRange(startId, endId);
            startRow.scrollIntoView({ block: 'center' });
        }
    }
}

// 9. Add a copy handler that force plain/text copy
logsEl.addEventListener("copy", function(e) {
    var text = window.getSelection().toString();
    e.clipboardData.setData('text/plain', text);
    e.preventDefault();
});

// 10. Add click event to handle custom hightling
logsEl.addEventListener('click', (e) => {
    const rowEl = e.target.closest('tr');
    if (!rowEl || !e.target.classList.contains("timestamp")) return;

    const ctrlOrMeta = e.ctrlKey || e.metaKey;
    const shiftKey = e.shiftKey;
    const rowId = getRowId(rowEl);

    // Prevent default link behavior
    e.preventDefault();
    e.stopPropagation();

    if (!ctrlOrMeta && !shiftKey) {
        // Normal click: select single row, set anchor
        startingAnchorId = rowId;
        highlightTimestampRange(startingAnchorId, startingAnchorId);
    } else if (shiftKey && startingAnchorId !== null) {
        // Shift+click: extend selection from anchor
        highlightTimestampRange(startingAnchorId, rowId);
    } else if (ctrlOrMeta) {
        // Ctrl/Cmd+click: new anchor (resets selection)
        startingAnchorId = rowId;
        highlightTimestampRange(startingAnchorId, startingAnchorId);
    }

    // Update our URL hash after every selection change
    const ids = Array.from(logsEl.querySelectorAll('tr.selected')).map(getRowId).sort();
    window.location.hash = ids.length ? 
        (ids.length === 1 ? `L${ids[0]}` : `L${ids[0]}-L${ids[ids.length-1]}`) : '';
});

// Helper function to get the ID of the given row
function getRowId(rowEl) {
    return rowEl.querySelector('a.timestamp').id; // "2025-12-12T21:28:09.6347029Z"
}

// Helper function to highlight (toggle the selected class) on the given timestamp range
function highlightTimestampRange(startId, endId) {
    const rows = Array.from(logsEl.querySelectorAll('tr')).filter(r => r.querySelector('.timestamp'));

    const startIndex = rows.findIndex(row => getRowId(row) === startId);
    const endIndex = rows.findIndex(row => getRowId(row) === endId);

    const start = Math.min(startIndex, endIndex);
    const end = Math.max(startIndex, endIndex);

    rows.forEach((row, index) => row.classList.toggle('selected', index >= start && index <= end));
}
