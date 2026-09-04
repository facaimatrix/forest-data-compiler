/* Forest Data Compiler — Forest Data Exchange desktop UI */

const state = {
  mode: 'compile', // compile | metadata
  step: 1,
  manifestPath: null,
  manifest: null,
  summary: null,
  folder: null,
  candidates: [],
  joinedOnly: true,
  includeUnregistered: false,
  recursive: false,
  format: 'auto',
  addSourceColumn: true,
  lastReport: null,
  geoMode: 'global', // global | bioregions | by_country
  selectedBioregions: [],
  selectedCountries: [],
  discoveredCountries: [],
  discoveredForestTypes: [],
  bioregionOptions: [],
  metadataFolder: null,
  metadataItems: [],
  metadataRecursive: false,
  metadataOverwrite: false,
  metadataContactEmail: '',
  metadataContactName: '',
  lastMetadataReport: null,
  metadataOpenAuthors: {},
  busy: false,
};

const STEPS = [
  { id: 1, label: '1. Manifest' },
  { id: 2, label: '2. Folder' },
  { id: 3, label: '3. Compile' },
];

const DEFAULT_BIOREGIONS = [
  'Tropical moist broadleaf forests',
  'Tropical dry broadleaf forests',
  'Tropical coniferous forests',
  'Temperate broadleaf & mixed forests',
  'Temperate coniferous forests',
  'Boreal forests/taiga',
  'Mediterranean forests',
  'Mangroves',
];

function api() {
  return window.__TAURI__;
}

async function invoke(cmd, args) {
  const t = api();
  if (!t?.core?.invoke) throw new Error('Tauri API not available');
  return t.core.invoke(cmd, args);
}

function normalizeDialogPath(result) {
  if (!result) return null;
  if (typeof result === 'string') return result;
  if (Array.isArray(result)) return result[0] || null;
  if (typeof result === 'object' && result.path) return result.path;
  return String(result);
}

async function openFile(filters) {
  const dialog = api()?.dialog;
  if (!dialog) throw new Error('Dialog plugin unavailable');
  const result = await dialog.open({ multiple: false, directory: false, filters });
  return normalizeDialogPath(result);
}

async function openFolder() {
  const dialog = api()?.dialog;
  if (!dialog) throw new Error('Dialog plugin unavailable');
  const result = await dialog.open({ multiple: false, directory: true });
  return normalizeDialogPath(result);
}

async function saveFile(defaultPath, filters) {
  const dialog = api()?.dialog;
  if (!dialog) throw new Error('Dialog plugin unavailable');
  const result = await dialog.save({ defaultPath, filters });
  return normalizeDialogPath(result);
}

function $(id) {
  return document.getElementById(id);
}

function showError(msg) {
  const bar = $('error-bar');
  if (!msg) {
    bar.hidden = true;
    bar.textContent = '';
    return;
  }
  bar.hidden = false;
  bar.textContent = msg;
}

function setLoading(on, msg) {
  const el = $('loading');
  if (!el) return;
  state.busy = !!on;
  if (on) {
    el.removeAttribute('hidden');
    el.hidden = false;
    if (msg) $('loading-msg').textContent = msg;
  } else {
    el.setAttribute('hidden', '');
    el.hidden = true;
    $('loading-msg').textContent = 'Working…';
  }
}

function escapeHtml(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function renderPills() {
  const host = $('step-pills');
  const modeTabs = `
    <button type="button" class="mode-tab ${state.mode === 'compile' ? 'active' : ''}" data-mode="compile">Compile project</button>
    <button type="button" class="mode-tab ${state.mode === 'metadata' ? 'active' : ''}" data-mode="metadata">Dataset metadata</button>
  `;
  const stepPills =
    state.mode === 'compile'
      ? STEPS.map((s) => {
          const disabled = s.id > 1 && !state.manifest ? 'disabled' : s.id > 2 && !state.folder ? 'disabled' : '';
          const active = s.id === state.step ? 'active' : '';
          return `<button type="button" class="step-pill ${active}" data-step="${s.id}" ${disabled}>${s.label}</button>`;
        }).join('')
      : '';
  host.innerHTML = `${modeTabs}${stepPills}`;
  host.querySelectorAll('.mode-tab').forEach((btn) => {
    btn.addEventListener('click', () => {
      state.mode = btn.dataset.mode;
      render();
    });
  });
  host.querySelectorAll('.step-pill').forEach((btn) => {
    btn.addEventListener('click', () => {
      const n = Number(btn.dataset.step);
      if (n === 1 || (n === 2 && state.manifest) || (n === 3 && state.folder)) {
        state.step = n;
        render();
      }
    });
  });
}

function selectedPaths() {
  return state.candidates.filter((c) => c.selected).map((c) => c.path);
}

function mergeUnique(...lists) {
  const out = [];
  for (const list of lists) {
    for (const item of list) {
      if (!out.some((x) => x.toLowerCase() === String(item).toLowerCase())) out.push(item);
    }
  }
  return out.sort((a, b) => a.localeCompare(b));
}

function geoPayload() {
  return {
    geo_mode: state.geoMode,
    bioregions: state.geoMode === 'bioregions' ? state.selectedBioregions : [],
    countries: state.geoMode === 'by_country' ? state.selectedCountries : [],
  };
}

function render() {
  showError(null);
  renderPills();
  const main = $('main');
  if (state.mode === 'metadata') main.innerHTML = viewMetadata();
  else if (state.step === 1) main.innerHTML = viewManifest();
  else if (state.step === 2) main.innerHTML = viewFolder();
  else main.innerHTML = viewCompile();
  bindStepHandlers();
}

function presentAttributes(attrs) {
  return Object.entries(attrs || {})
    .filter(([, on]) => on)
    .map(([key]) => key.replace(/_/g, ' '))
    .join(', ') || '—';
}

function yearLabel(range) {
  if (!range) return '—';
  const start = range.year_start ?? 'any';
  const end = range.year_end ?? 'present';
  return `${start} – ${end}`;
}

function emptyCoauthor() {
  return { author_name: '', author_email: '', role: 'co_author', affiliation: '', author_order: null };
}

function authorSummary(m) {
  const people = m.coauthors || [];
  if (!people.length) return 'No authors yet';
  return people
    .map((p) => p.author_name || p.author_email || 'unnamed')
    .join(', ');
}

function authorEditorHtml(item, i) {
  const people = item.metadata.coauthors || [];
  const rows = people
    .map((p, j) => `
      <tr>
        <td class="tiny">${j + 1}</td>
        <td><input type="text" class="author-field" data-idx="${i}" data-author="${j}" data-key="author_name" value="${escapeHtml(p.author_name || '')}" placeholder="Name"/></td>
        <td><input type="text" class="author-field" data-idx="${i}" data-author="${j}" data-key="author_email" value="${escapeHtml(p.author_email || '')}" placeholder="email@example.org"/></td>
        <td><input type="text" class="author-field" data-idx="${i}" data-author="${j}" data-key="affiliation" value="${escapeHtml(p.affiliation || '')}" placeholder="Affiliation"/></td>
        <td>
          <select class="author-field select" data-idx="${i}" data-author="${j}" data-key="role">
            <option value="corresponding" ${p.role === 'corresponding' ? 'selected' : ''}>Corresponding</option>
            <option value="co_author" ${p.role !== 'corresponding' ? 'selected' : ''}>Co-author</option>
          </select>
        </td>
        <td>
          <button type="button" class="btn btn-ghost author-move" data-idx="${i}" data-author="${j}" data-dir="-1" ${j === 0 ? 'disabled' : ''}>↑</button>
          <button type="button" class="btn btn-ghost author-move" data-idx="${i}" data-author="${j}" data-dir="1" ${j === people.length - 1 ? 'disabled' : ''}>↓</button>
          <button type="button" class="btn btn-ghost author-remove" data-idx="${i}" data-author="${j}">Remove</button>
        </td>
      </tr>
    `)
    .join('');
  return `
    <div class="author-box">
      <div class="row" style="margin-bottom:.45rem">
        <strong class="tiny" style="color:var(--green-dark)">Authors / contacts</strong>
        <button type="button" class="btn btn-secondary author-add" data-idx="${i}">Add person</button>
      </div>
      ${
        people.length
          ? `<table class="files author-table">
              <thead><tr><th>#</th><th>Name</th><th>Email</th><th>Affiliation</th><th>Role</th><th></th></tr></thead>
              <tbody>${rows}</tbody>
            </table>`
          : `<p class="tiny">Add the people who should be contacted or listed as authors for this dataset.</p>`
      }
    </div>
  `;
}

function viewMetadata() {
  const items = state.metadataItems;
  const report = state.lastMetadataReport;
  const rows = items
    .map((item, i) => {
      const m = item.metadata || {};
      const geo = m.geography || {};
      const attrs = presentAttributes(m.attributes);
      const notes = (m.notes || []).map((n) => `<div class="tiny">${escapeHtml(n)}</div>`).join('');
      const exists = item.sidecar_exists
        ? `<span class="badge badge-warn">Has sidecar</span>`
        : `<span class="badge badge-muted">New</span>`;
      const gfb = m.source?.looks_like_gfb3
        ? `<span class="badge badge-ok">GFB3</span>`
        : `<span class="badge badge-err">Not GFB3</span>`;
      const open = !!state.metadataOpenAuthors[i];
      return `
        <tr>
          <td><input type="checkbox" class="meta-check" data-idx="${i}" ${item.selected !== false ? 'checked' : ''}/></td>
          <td>
            <div style="font-weight:600">${escapeHtml(m.source?.file_name || '—')}</div>
            <div class="tiny">${escapeHtml(m.source?.path || '')}</div>
            <div class="tiny">Sidecar: ${escapeHtml(item.sidecar_path || '')}</div>
          </td>
          <td>${gfb} ${exists}</td>
          <td>${m.num_plots ?? '—'} / ${m.num_trees ?? '—'}</td>
          <td>${escapeHtml(yearLabel(m.year_range))}</td>
          <td class="tiny">${escapeHtml((geo.countries || []).join(', ') || '—')}</td>
          <td>
            <div class="tiny">${escapeHtml(authorSummary(m))}</div>
            <button type="button" class="btn btn-secondary author-toggle" data-idx="${i}" style="margin-top:.35rem">
              ${open ? 'Hide authors' : 'Edit authors'}
            </button>
          </td>
          <td class="tiny">${escapeHtml((m.forest_types || []).join(', ') || '—')}</td>
          <td class="tiny">${escapeHtml(attrs)}${notes}</td>
        </tr>
        ${open ? `<tr class="author-row"><td></td><td colspan="8">${authorEditorHtml(item, i)}</td></tr>` : ''}
      `;
    })
    .join('');

  return `
    <section class="panel">
      <h2>Generate dataset metadata</h2>
      <p class="lede">
        For GFB3 files ingested before Forest Data Exchange, this writes a
        <code>{name}.metadata.json</code> sidecar next to each table. The compiler
        reads those files when matching, and the JSON matches the website
        dataset record (attributes, geography, forest types, census years, plot counts).
      </p>
      <div class="row">
        <button type="button" class="btn btn-primary" id="btn-pick-meta-folder">Choose folder…</button>
        <button type="button" class="btn btn-secondary" id="btn-rescan-meta" ${state.metadataFolder ? '' : 'disabled'}>Rescan</button>
        <div class="path-box" title="${escapeHtml(state.metadataFolder || '')}">${escapeHtml(state.metadataFolder || 'No folder selected')}</div>
      </div>
      <div class="toggles">
        <label class="toggle"><input type="checkbox" id="opt-meta-recursive" ${state.metadataRecursive ? 'checked' : ''}/> Scan subfolders</label>
        <label class="toggle"><input type="checkbox" id="opt-meta-overwrite" ${state.metadataOverwrite ? 'checked' : ''}/> Overwrite existing sidecars</label>
      </div>
      <div class="row" style="margin-top:.5rem">
        <label class="tiny" style="flex:1">Contributor name
          <input type="text" id="meta-contact-name" value="${escapeHtml(state.metadataContactName)}" placeholder="Optional — applied to written files"/>
        </label>
        <label class="tiny" style="flex:1">Contributor email
          <input type="text" id="meta-contact-email" value="${escapeHtml(state.metadataContactEmail)}" placeholder="Optional — needed to register on the website"/>
        </label>
      </div>
      ${
        items.length
          ? `<div class="table-wrap" style="margin-top:1rem">
              <table class="files">
                <thead>
                  <tr>
                    <th></th><th>File</th><th>Status</th><th>Plots / trees</th>
                    <th>Years</th><th>Countries</th><th>Authors</th><th>Forest types</th><th>Attributes / notes</th>
                  </tr>
                </thead>
                <tbody>${rows}</tbody>
              </table>
            </div>
            <div class="actions">
              <span class="muted">${items.length} file(s) inspected</span>
              <div class="row">
                <button type="button" class="btn btn-secondary" id="btn-export-authors">Export author directory…</button>
                <button type="button" class="btn btn-primary" id="btn-write-meta">Write selected metadata JSON</button>
              </div>
            </div>`
          : state.metadataFolder
            ? `<p class="notice">No supported tables in this folder (CSV / TSV / XLSX / Parquet).</p>`
            : ''
      }
      ${
        report
          ? `<div class="notice" style="margin-top:1rem">
               Wrote ${report.written.length} sidecar(s)
               ${report.skipped.length ? ` · skipped ${report.skipped.length}` : ''}
               ${report.errors.length ? ` · ${report.errors.length} error(s)` : ''}.
               ${report.written.slice(0, 8).map((p) => `<div class="tiny">${escapeHtml(p)}</div>`).join('')}
             </div>`
          : ''
      }
    </section>
  `;
}

function viewManifest() {
  const s = state.summary;
  return `
    <section class="panel">
      <h2>Load compile manifest</h2>
      <p class="lede">
        Download the requirements JSON from Forest Data Exchange Admin → Compiled Data
        (named <code>forest-data-exchange-compile-manifest-…json</code>), then open it here.
        The app uses it to filter local GFB3 files for this project.
      </p>
      <div class="row">
        <button type="button" class="btn btn-primary" id="btn-pick-manifest">Choose JSON…</button>
        <div class="path-box" title="${escapeHtml(state.manifestPath || '')}">${escapeHtml(state.manifestPath || 'No file selected')}</div>
      </div>
      ${s ? manifestSummaryHtml(s) : ''}
      <div class="actions">
        <span class="muted">Schema: forest-data-exchange.compile_manifest v1</span>
        <button type="button" class="btn btn-primary" id="btn-next-2" ${s ? '' : 'disabled'}>Continue →</button>
      </div>
    </section>
  `;
}

function manifestGeographyLabel(s) {
  switch (s.geography_scope) {
    case 'continental':
      return `Continents: ${(s.continents || []).join(', ') || 'any'}`;
    case 'ecoregion':
    case 'ecoregions':
    case 'bioregions':
      return `Bioregions: ${(s.ecoregions || []).join(', ') || 'any'}`;
    case 'countries':
    case 'country':
      return `Countries: ${(s.countries || []).join(', ') || 'any'}`;
    case 'shapefile':
    case 'extent':
      return `Custom extent: ${s.extent_shapefile_name || 'shapefile'}`;
    default:
      return 'Global (no geographic filter)';
  }
}

function manifestSummaryHtml(s) {
  const mandatory = (s.mandatory || []).map((a) => `<span class="chip">${escapeHtml(a.label)}</span>`).join('') || '<span class="tiny">None listed</span>';
  const ideal = (s.ideal || []).map((a) => `<span class="chip ideal">${escapeHtml(a.label)}</span>`).join('') || '<span class="tiny">None</span>';
  const forestTypes = (s.forest_types || []).length
    ? s.forest_types.map((f) => `<span class="chip">${escapeHtml(f)}</span>`).join('')
    : '<span class="tiny">All</span>';
  const mapProducts = (s.map_products || []).length
    ? s.map_products.map((p) => `<span class="chip ideal">${escapeHtml(p.label)}</span>`).join('')
    : '';

  return `
    <div class="meta-grid">
      <div class="meta-card"><div class="label">Project</div><div class="value">${escapeHtml(s.project_title)}</div></div>
      <div class="meta-card"><div class="label">PI</div><div class="value">${escapeHtml(s.pi_name || s.pi_email || '—')}</div></div>
      <div class="meta-card"><div class="label">Status</div><div class="value">${escapeHtml(s.status || '—')}</div></div>
      <div class="meta-card"><div class="label">Join rate</div><div class="value">${s.join_percent != null ? s.join_percent + '%' : '—'} <span class="tiny">(${s.joined_owners} joined / ${s.pending_owners} pending)</span></div></div>
      <div class="meta-card"><div class="label">Suitable datasets</div><div class="value">${s.suitable_count}</div></div>
      <div class="meta-card"><div class="label">Project geography</div><div class="value" style="font-size:.85rem">${escapeHtml(manifestGeographyLabel(s))}</div></div>
      <div class="meta-card"><div class="label">Census years</div><div class="value">${escapeHtml(s.year_range_label || 'any')}</div></div>
    </div>
    <div>
      <div class="tiny" style="font-weight:700;margin-bottom:.25rem">Mandatory attributes${s.enforces_mandatory_attributes ? '' : ' (not enforced by this project)'}</div>
      <div class="chips">${mandatory}</div>
    </div>
    <div style="margin-top:.75rem">
      <div class="tiny" style="font-weight:700;margin-bottom:.25rem">Ideal attributes (ranking only)</div>
      <div class="chips">${ideal}</div>
    </div>
    <div style="margin-top:.75rem">
      <div class="tiny" style="font-weight:700;margin-bottom:.25rem">Forest types</div>
      <div class="chips">${forestTypes}</div>
    </div>
    ${
      mapProducts
        ? `<div style="margin-top:.75rem">
             <div class="tiny" style="font-weight:700;margin-bottom:.25rem">Map products (project deliverables, not built here)</div>
             <div class="chips">${mapProducts}</div>
           </div>`
        : ''
    }
    ${
      (s.unenforced || []).length
        ? `<div class="notice">${s.unenforced.map((u) => `<div>${escapeHtml(u)}</div>`).join('')}</div>`
        : ''
    }
  `;
}

function geoFilterHtml() {
  const bioOpts = (state.bioregionOptions.length ? state.bioregionOptions : DEFAULT_BIOREGIONS);
  const countryOpts = state.discoveredCountries;

  const bioList =
    state.geoMode === 'bioregions'
      ? `<div class="geo-checklist" id="bio-list">
          ${bioOpts
            .map(
              (b) => `<label><input type="checkbox" class="bio-check" value="${escapeHtml(b)}" ${
                state.selectedBioregions.some((x) => x.toLowerCase() === b.toLowerCase()) ? 'checked' : ''
              }/> ${escapeHtml(b)}</label>`,
            )
            .join('')}
        </div>`
      : '';

  const countryList =
    state.geoMode === 'by_country'
      ? countryOpts.length
        ? `<div class="geo-checklist" id="country-list">
            ${countryOpts
              .map(
                (c) => `<label><input type="checkbox" class="country-check" value="${escapeHtml(c)}" ${
                  state.selectedCountries.some((x) => x.toLowerCase() === c.toLowerCase()) ? 'checked' : ''
                }/> ${escapeHtml(c)}</label>`,
              )
              .join('')}
          </div>
          <p class="tiny" style="margin-top:.4rem">Countries discovered from the Country column in scanned files. Rescan after changing the folder.</p>`
        : `<p class="tiny">No Country values found yet. Scan a folder of GFB3 files that include a Country column, then select countries here.</p>`
      : '';

  return `
    <div class="geo-box">
      <h3>Geographic filter</h3>
      <div class="geo-modes">
        <label><input type="radio" name="geo-mode" value="global" ${state.geoMode === 'global' ? 'checked' : ''}/> Global</label>
        <label><input type="radio" name="geo-mode" value="bioregions" ${state.geoMode === 'bioregions' ? 'checked' : ''}/> Bioregions</label>
        <label><input type="radio" name="geo-mode" value="by_country" ${state.geoMode === 'by_country' ? 'checked' : ''}/> By country</label>
      </div>
      ${bioList}
      ${countryList}
      <p class="tiny" style="margin-top:.5rem">
        Applied when matching files and when merging rows (Country / Bioregion–Ecoregion columns).
        This narrows further; the project scope from the manifest${
          state.summary ? ` (${escapeHtml(manifestGeographyLabel(state.summary))})` : ''
        } always applies.
      </p>
    </div>
  `;
}

function viewFolder() {
  const rows = state.candidates.map((c, i) => candidateRow(c, i)).join('');
  return `
    <section class="panel">
      <h2>Select local GFB3 folder</h2>
      <p class="lede">
        Point at the folder where contributor tree-level files live. Matching uses
        registered dataset names from the manifest, then checks mandatory attributes and geography.
      </p>
      <div class="row">
        <button type="button" class="btn btn-primary" id="btn-pick-folder">Choose folder…</button>
        <button type="button" class="btn btn-secondary" id="btn-rescan" ${state.folder ? '' : 'disabled'}>Rescan</button>
        <div class="path-box" title="${escapeHtml(state.folder || '')}">${escapeHtml(state.folder || 'No folder selected')}</div>
      </div>
      ${geoFilterHtml()}
      <div class="toggles">
        <label class="toggle"><input type="checkbox" id="opt-joined" ${state.joinedOnly ? 'checked' : ''}/> Joined owners only</label>
        <label class="toggle"><input type="checkbox" id="opt-unreg" ${state.includeUnregistered ? 'checked' : ''}/> Include unregistered GFB3-looking files</label>
        <label class="toggle"><input type="checkbox" id="opt-recursive" ${state.recursive ? 'checked' : ''}/> Scan subfolders</label>
      </div>
      ${
        state.folder
          ? `<p class="muted">${state.candidates.length} candidate(s) · ${selectedPaths().length} selected</p>
             <div class="row" style="margin-top:.5rem">
               <button type="button" class="btn btn-ghost" id="btn-select-all">Select all eligible</button>
               <button type="button" class="btn btn-ghost" id="btn-select-none">Clear selection</button>
             </div>
             <div style="overflow:auto">
               <table class="files">
                 <thead>
                   <tr>
                     <th></th>
                     <th>File</th>
                     <th>Match</th>
                     <th>Owner</th>
                     <th>Ideal</th>
                     <th>Notes</th>
                   </tr>
                 </thead>
                 <tbody>${rows || `<tr><td colspan="6" class="muted">No matching files in this folder.</td></tr>`}</tbody>
               </table>
             </div>`
          : ''
      }
      <div class="actions">
        <button type="button" class="btn btn-secondary" id="btn-back-1">← Back</button>
        <button type="button" class="btn btn-primary" id="btn-next-3" ${selectedPaths().length ? '' : 'disabled'}>Compile selected →</button>
      </div>
    </section>
  `;
}

function candidateRow(c, i) {
  const joined = c.registered?.owner_joined;
  const matchBadge =
    c.reason === 'registered_name'
      ? `<span class="badge badge-ok">Registered</span>`
      : c.reason === 'sidecar_metadata'
        ? `<span class="badge badge-ok">Metadata</span>`
        : `<span class="badge badge-muted">Columns</span>`;
  const geoBadge = c.geography_ok
    ? ''
    : ` <span class="badge badge-warn">Geo</span>`;
  const missBadge =
    c.missing_mandatory?.length && state.summary?.enforces_mandatory_attributes
      ? ` <span class="badge badge-err">Attrs</span>`
      : '';
  const forestBadge = c.forest_type_ok === false ? ` <span class="badge badge-warn">Forest</span>` : '';
  const owner = c.registered
    ? `${escapeHtml(c.registered.contributor_name || c.registered.contributor_email || '—')}${
        joined ? ' <span class="badge badge-ok">joined</span>' : ' <span class="badge badge-warn">pending</span>'
      }`
    : '<span class="tiny">—</span>';
  const geoBits = [];
  if (c.countries_found?.length) geoBits.push(`Countries: ${c.countries_found.join(', ')}`);
  if (c.bioregions_found?.length) geoBits.push(`Bioregions: ${c.bioregions_found.join(', ')}`);
  if (c.forest_types_found?.length) geoBits.push(`Forest types: ${c.forest_types_found.join(', ')}`);
  const notes = [
    ...geoBits,
    ...(c.warnings || []),
    ...(c.error ? [`Read error: ${c.error}`] : []),
  ]
    .map((w) => `<div class="tiny">${escapeHtml(w)}</div>`)
    .join('');

  return `
    <tr>
      <td><input type="checkbox" data-idx="${i}" class="row-check" ${c.selected ? 'checked' : ''} ${c.error ? 'disabled' : ''}/></td>
      <td>
        <div style="font-weight:600">${escapeHtml(c.file_name)}</div>
        <div class="tiny">${escapeHtml(c.path)}</div>
      </td>
      <td>${matchBadge}${geoBadge}${forestBadge}${missBadge}</td>
      <td>${owner}</td>
      <td>${c.ideal_hits}/${c.ideal_total}</td>
      <td>${notes || '<span class="tiny">—</span>'}</td>
    </tr>
  `;
}

function viewCompile() {
  const n = selectedPaths().length;
  const report = state.lastReport;
  const geoLabel =
    state.geoMode === 'bioregions'
      ? `Bioregions (${state.selectedBioregions.length ? state.selectedBioregions.join(', ') : 'any'})`
      : state.geoMode === 'by_country'
        ? `By country (${state.selectedCountries.length ? state.selectedCountries.join(', ') : 'any'})`
        : 'Global';
  return `
    <section class="panel">
      <h2>Compile dataset</h2>
      <p class="lede">
        Merge the ${n} selected file(s) into one compiled product for
        <strong>${escapeHtml(state.summary?.project_title || 'this project')}</strong>.
        Upload the result in Forest Data Exchange Admin → Compiled Data → Manual upload.
      </p>
      <div class="meta-grid">
        <div class="meta-card"><div class="label">Selected files</div><div class="value">${n}</div></div>
        <div class="meta-card"><div class="label">Geography filter</div><div class="value" style="font-size:.85rem">${escapeHtml(geoLabel)}</div></div>
        <div class="meta-card"><div class="label">Project id</div><div class="value" style="font-size:.8rem">${escapeHtml(state.summary?.project_id || '')}</div></div>
      </div>
      <div class="row" style="margin:1rem 0">
        <label class="toggle">Output format
          <select class="select" id="opt-format">
            <option value="auto" ${state.format === 'auto' ? 'selected' : ''}>Auto (CSV if all CSV, else XLSX)</option>
            <option value="csv" ${state.format === 'csv' ? 'selected' : ''}>CSV (merged)</option>
            <option value="xlsx" ${state.format === 'xlsx' ? 'selected' : ''}>XLSX (merged)</option>
            <option value="parquet" ${state.format === 'parquet' ? 'selected' : ''}>Parquet (merged)</option>
            <option value="zip" ${state.format === 'zip' ? 'selected' : ''}>ZIP originals</option>
          </select>
        </label>
        <label class="toggle"><input type="checkbox" id="opt-source-col" ${state.addSourceColumn ? 'checked' : ''}/> Add compile_source column</label>
      </div>
      <div class="actions">
        <button type="button" class="btn btn-secondary" id="btn-back-2">← Back</button>
        <button type="button" class="btn btn-primary" id="btn-compile" ${n ? '' : 'disabled'}>Compile &amp; save…</button>
      </div>
      ${
        report
          ? `<div class="success-box">
               <strong>Compiled successfully</strong>
               <div style="margin-top:.4rem">${escapeHtml(report.output_path)}</div>
               <div class="tiny" style="margin-top:.35rem;color:inherit;opacity:.85">
                 ${report.source_count} source(s) · ${report.total_rows} rows · format ${escapeHtml(report.format)}
                 · report: ${escapeHtml(report.output_path)}.compile-report.json
               </div>
             </div>`
          : ''
      }
    </section>
  `;
}

async function scanMetadataFolder() {
  if (!state.metadataFolder) return;
  try {
    showError(null);
    setLoading(true, 'Reading datasets…');
    const result = await invoke('inspect_folder_metadata', {
      input: {
        folder: state.metadataFolder,
        recursive: state.metadataRecursive,
      },
    });
    const previous = new Map(
      (state.metadataItems || []).map((item) => [item.metadata?.source?.path, item]),
    );
    state.metadataItems = (result || []).map((item) => {
      const prev = previous.get(item.metadata?.source?.path);
      if (prev?.metadata) {
        const incoming = item.metadata.coauthors || [];
        const edited = prev.metadata.coauthors || [];
        item.metadata.coauthors = edited.length ? edited : incoming;
        item.metadata.contributor_name =
          prev.metadata.contributor_name || item.metadata.contributor_name;
        item.metadata.contributor_email =
          prev.metadata.contributor_email || item.metadata.contributor_email;
        item.selected = prev.selected;
      } else {
        item.selected = true;
        if (!item.metadata.coauthors) item.metadata.coauthors = [];
      }
      return item;
    });
    state.lastMetadataReport = null;
    render();
  } catch (e) {
    showError(String(e?.message || e));
  } finally {
    setLoading(false);
  }
}

function selectedMetadata() {
  return state.metadataItems.filter((item) => item.selected !== false);
}

function normalizeCoauthors(people) {
  return (people || [])
    .map((p, i) => ({
      author_name: (p.author_name || '').trim(),
      author_email: (p.author_email || '').trim() || null,
      affiliation: (p.affiliation || '').trim() || null,
      role: p.role === 'corresponding' ? 'corresponding' : 'co_author',
      author_order: i + 1,
    }))
    .filter((p) => p.author_name || p.author_email);
}

function bindStepHandlers() {
  $('btn-cancel-loading')?.addEventListener('click', () => setLoading(false));

  $('btn-pick-meta-folder')?.addEventListener('click', async () => {
    try {
      const folder = await openFolder();
      if (!folder) return;
      state.metadataFolder = folder;
      await scanMetadataFolder();
    } catch (e) {
      showError(String(e?.message || e));
      setLoading(false);
    }
  });
  $('btn-rescan-meta')?.addEventListener('click', scanMetadataFolder);
  $('opt-meta-recursive')?.addEventListener('change', async (e) => {
    state.metadataRecursive = e.target.checked;
    if (state.metadataFolder) await scanMetadataFolder();
  });
  $('opt-meta-overwrite')?.addEventListener('change', (e) => {
    state.metadataOverwrite = e.target.checked;
  });
  $('meta-contact-name')?.addEventListener('input', (e) => {
    state.metadataContactName = e.target.value;
  });
  $('meta-contact-email')?.addEventListener('input', (e) => {
    state.metadataContactEmail = e.target.value;
  });
  document.querySelectorAll('.meta-check').forEach((cb) => {
    cb.addEventListener('change', () => {
      const i = Number(cb.dataset.idx);
      if (state.metadataItems[i]) state.metadataItems[i].selected = cb.checked;
    });
  });
  document.querySelectorAll('.author-toggle').forEach((btn) => {
    btn.addEventListener('click', () => {
      const i = Number(btn.dataset.idx);
      state.metadataOpenAuthors[i] = !state.metadataOpenAuthors[i];
      render();
    });
  });
  document.querySelectorAll('.author-add').forEach((btn) => {
    btn.addEventListener('click', () => {
      const i = Number(btn.dataset.idx);
      const item = state.metadataItems[i];
      if (!item?.metadata) return;
      if (!item.metadata.coauthors) item.metadata.coauthors = [];
      item.metadata.coauthors.push(emptyCoauthor());
      state.metadataOpenAuthors[i] = true;
      render();
    });
  });
  document.querySelectorAll('.author-remove').forEach((btn) => {
    btn.addEventListener('click', () => {
      const i = Number(btn.dataset.idx);
      const j = Number(btn.dataset.author);
      const people = state.metadataItems[i]?.metadata?.coauthors;
      if (!people) return;
      people.splice(j, 1);
      render();
    });
  });
  document.querySelectorAll('.author-move').forEach((btn) => {
    btn.addEventListener('click', () => {
      const i = Number(btn.dataset.idx);
      const j = Number(btn.dataset.author);
      const dir = Number(btn.dataset.dir);
      const people = state.metadataItems[i]?.metadata?.coauthors;
      const next = j + dir;
      if (!people || next < 0 || next >= people.length) return;
      const [row] = people.splice(j, 1);
      people.splice(next, 0, row);
      render();
    });
  });
  document.querySelectorAll('.author-field').forEach((el) => {
    el.addEventListener('input', () => {
      const i = Number(el.dataset.idx);
      const j = Number(el.dataset.author);
      const key = el.dataset.key;
      const person = state.metadataItems[i]?.metadata?.coauthors?.[j];
      if (!person) return;
      person[key] = el.value;
    });
    el.addEventListener('change', () => {
      const i = Number(el.dataset.idx);
      const j = Number(el.dataset.author);
      const key = el.dataset.key;
      const person = state.metadataItems[i]?.metadata?.coauthors?.[j];
      if (!person) return;
      person[key] = el.value;
    });
  });
  $('btn-export-authors')?.addEventListener('click', async () => {
    const items = selectedMetadata().map((item) => {
      const metadata = { ...item.metadata };
      metadata.coauthors = normalizeCoauthors(metadata.coauthors);
      return metadata;
    });
    if (!items.length) {
      showError('Select at least one file.');
      return;
    }
    try {
      const output = await saveFile('forest-data-exchange-author-directory.json', [
        { name: 'Author directory', extensions: ['json'] },
      ]);
      if (!output) return;
      setLoading(true, 'Writing author directory…');
      const path = await invoke('export_author_directory', {
        input: { items, output_path: output },
      });
      state.lastMetadataReport = {
        written: [path],
        skipped: [],
        errors: [],
      };
      render();
    } catch (e) {
      showError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  });
  $('btn-write-meta')?.addEventListener('click', async () => {
    const items = selectedMetadata().map((item) => {
      const metadata = { ...item.metadata };
      metadata.coauthors = normalizeCoauthors(metadata.coauthors);
      return metadata;
    });
    if (!items.length) {
      showError('Select at least one file.');
      return;
    }
    try {
      showError(null);
      setLoading(true, 'Writing metadata JSON…');
      const report = await invoke('write_dataset_metadata', {
        input: {
          items,
          overwrite: state.metadataOverwrite,
          contributor_email: state.metadataContactEmail || null,
          contributor_name: state.metadataContactName || null,
        },
      });
      state.lastMetadataReport = report;
      await scanMetadataFolder();
      state.lastMetadataReport = report;
      render();
    } catch (e) {
      showError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  });

  const pickManifest = $('btn-pick-manifest');
  if (pickManifest) {
    pickManifest.addEventListener('click', async () => {
      try {
        showError(null);
        const path = await openFile([{ name: 'Compile Manifest', extensions: ['json'] }]);
        if (!path) return;
        setLoading(true, 'Loading manifest…');
        const summary = await invoke('load_manifest', { path });
        state.manifestPath = summary.path;
        state.summary = summary;
        state.manifest = summary.raw;
        state.bioregionOptions = summary.bioregion_options || DEFAULT_BIOREGIONS;
        // Pre-fill the optional narrowing lists with the project's own geography,
        // so switching mode starts from the project scope rather than empty.
        state.selectedBioregions = [...(summary.ecoregions || [])];
        state.selectedCountries = [...(summary.countries || [])];
        state.discoveredCountries = [...(summary.countries || [])];
        state.discoveredForestTypes = [];
        state.geoMode = 'global';
        state.candidates = [];
        state.folder = null;
        state.lastReport = null;
        state.step = 1;
        render();
      } catch (e) {
        showError(String(e?.message || e));
      } finally {
        setLoading(false);
      }
    });
  }

  $('btn-next-2')?.addEventListener('click', () => {
    state.step = 2;
    render();
  });
  $('btn-back-1')?.addEventListener('click', () => {
    state.step = 1;
    render();
  });
  $('btn-back-2')?.addEventListener('click', () => {
    state.step = 2;
    render();
  });
  $('btn-next-3')?.addEventListener('click', () => {
    state.step = 3;
    state.lastReport = null;
    render();
  });

  const rescan = async () => {
    if (!state.folder || !state.manifest) return;
    try {
      showError(null);
      setLoading(true, 'Scanning folder…');
      const result = await invoke('scan_folder', {
        input: {
          folder: state.folder,
          manifest: state.manifest,
          joined_owners_only: state.joinedOnly,
          include_unregistered: state.includeUnregistered,
          recursive: state.recursive,
          ...geoPayload(),
        },
      });
      state.candidates = result.candidates || [];
      state.discoveredCountries = mergeUnique(
        state.summary?.countries || [],
        result.discovered_countries || [],
      );
      state.discoveredForestTypes = result.discovered_forest_types || [];
      if (result.bioregion_options?.length) {
        state.bioregionOptions = result.bioregion_options;
      }
      render();
    } catch (e) {
      showError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  };

  $('btn-pick-folder')?.addEventListener('click', async () => {
    try {
      const folder = await openFolder();
      if (!folder) return;
      state.folder = folder;
      await rescan();
    } catch (e) {
      showError(String(e?.message || e));
      setLoading(false);
    }
  });
  $('btn-rescan')?.addEventListener('click', rescan);

  document.querySelectorAll('input[name="geo-mode"]').forEach((radio) => {
    radio.addEventListener('change', async () => {
      state.geoMode = radio.value;
      render();
      if (state.folder) await rescan();
    });
  });

  document.querySelectorAll('.bio-check').forEach((cb) => {
    cb.addEventListener('change', async () => {
      state.selectedBioregions = [...document.querySelectorAll('.bio-check:checked')].map((el) => el.value);
      if (state.folder) await rescan();
    });
  });

  document.querySelectorAll('.country-check').forEach((cb) => {
    cb.addEventListener('change', async () => {
      state.selectedCountries = [...document.querySelectorAll('.country-check:checked')].map((el) => el.value);
      if (state.folder) await rescan();
    });
  });

  $('opt-joined')?.addEventListener('change', async (e) => {
    state.joinedOnly = e.target.checked;
    await rescan();
  });
  $('opt-unreg')?.addEventListener('change', async (e) => {
    state.includeUnregistered = e.target.checked;
    await rescan();
  });
  $('opt-recursive')?.addEventListener('change', async (e) => {
    state.recursive = e.target.checked;
    await rescan();
  });

  document.querySelectorAll('.row-check').forEach((cb) => {
    cb.addEventListener('change', () => {
      const i = Number(cb.dataset.idx);
      if (state.candidates[i]) state.candidates[i].selected = cb.checked;
      const next = $('btn-next-3');
      if (next) next.disabled = selectedPaths().length === 0;
    });
  });

  $('btn-select-all')?.addEventListener('click', () => {
    state.candidates.forEach((c) => {
      c.selected =
        !c.error &&
        (c.missing_mandatory.length === 0 || !state.summary?.enforces_mandatory_attributes) &&
        c.geography_ok &&
        c.forest_type_ok !== false;
    });
    render();
  });
  $('btn-select-none')?.addEventListener('click', () => {
    state.candidates.forEach((c) => {
      c.selected = false;
    });
    render();
  });

  $('opt-format')?.addEventListener('change', (e) => {
    state.format = e.target.value;
  });
  $('opt-source-col')?.addEventListener('change', (e) => {
    state.addSourceColumn = e.target.checked;
  });

  $('btn-compile')?.addEventListener('click', async () => {
    try {
      showError(null);
      const paths = selectedPaths();
      if (!paths.length) return;

      setLoading(true, 'Preparing…');
      const suggested = await invoke('suggest_output_name', {
        manifest: state.manifest,
        format: state.format,
      });
      setLoading(false);

      const ext =
        state.format === 'csv'
          ? ['csv']
          : state.format === 'parquet'
            ? ['parquet']
            : state.format === 'zip'
              ? ['zip']
              : state.format === 'xlsx'
                ? ['xlsx']
                : ['csv', 'xlsx', 'parquet', 'zip'];

      const output = await saveFile(suggested, [
        { name: 'Compiled dataset', extensions: ext },
      ]);
      if (!output) return;

      setLoading(true, 'Compiling…');
      const report = await invoke('compile_selection', {
        input: {
          paths,
          output_path: output,
          manifest: state.manifest,
          format: state.format,
          add_source_column: state.addSourceColumn,
          ...geoPayload(),
        },
      });
      state.lastReport = report;
      render();
    } catch (e) {
      showError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  });
}

function boot() {
  $('btn-cancel-loading')?.addEventListener('click', () => setLoading(false));
  let tries = 0;
  const tick = () => {
    tries += 1;
    if (api()?.core?.invoke) {
      render();
      return;
    }
    if (tries > 40) {
      $('main').innerHTML = `<section class="panel"><h2>Tauri API missing</h2><p class="lede">Open this app with <code>cargo tauri dev</code> (see <code>dev.ps1</code>), not as a plain browser page.</p></section>`;
      return;
    }
    setTimeout(tick, 50);
  };
  tick();
}

document.addEventListener('DOMContentLoaded', boot);
