/* Conduit — 8-bit pixel logo engine (double-pipe, themeable, conduit cues).
 * Single source of truth for the showcase page AND the PNG exporter.
 * Parallel straight conduits with shaded + DITHERED tube walls (top-lit
 * highlight -> dark shadow edge) and bright packets flowing through.
 * Optional "this is a pipe" cues: tube thickness, coupling collars, end nodes.
 * Native art grid is 32x32 logical pixels; clean 2:1/4:1 downscale to every
 * icon size (16/32/64/128/256/512), draws crisp at any size.
 */
(function (root) {
  var N = 32;       // native grid; halves cleanly to 16, quarters to 8, etc.
  var G = 2;        // gap between parallel lanes
  var COREW = 3;    // packet core width along the flow

  // Themes carry TONES so tube thickness is adjustable:
  //   hi=top highlight, lite/drk=dithered body tones, edge=shadow edge,
  //   intr=channel interior, lanes[]=packet {core,glow}, collar/node accents.
  var THEMES = {
    mono: {
      label: 'Mono', sub: 'white + black, 1-bit grain',
      frameBg: '#000000', iconBg: '#000000', intr: '#0c0e11',
      hi: '#ffffff', lite: { d: ['#d4d7da', '#9aa0a6'] }, drk: { d: ['#5b6166', '#383d42'] }, edge: '#24282c',
      collar: '#ffffff', node: '#e8eaed',
      lanes: [{ core: '#ffffff', glow: '#aab0b5' }, { core: '#b9bec2', glow: '#6b7176' }]
    },
    amber: {
      label: 'Amber', sub: 'CRT phosphor warmth',
      frameBg: '#0b0704', iconBg: '#120c05', intr: '#140d05',
      hi: '#ffd591', lite: { d: ['#d99a4e', '#9c6a2c'] }, drk: { d: ['#8a5e27', '#4d3415'] }, edge: '#36240f',
      collar: '#ffe0a6', node: '#ffce7a',
      lanes: [{ core: '#ffce7a', glow: '#c98a3a' }, { core: '#ff9f4d', glow: '#b5641f' }]
    },
    signal: {
      label: 'Signal', sub: 'cyan + magenta (brand)',
      frameBg: '#0a0d12', iconBg: '#0d1117', intr: '#08181c',
      hi: '#8af0f6', lite: { d: ['#3fbcc3', '#1f868c'] }, drk: { d: ['#1c787e', '#0f3d41'] }, edge: '#0b2a2e',
      collar: '#aef6fb', node: '#5ef0f7',
      lanes: [{ core: '#5ef0f7', glow: '#1fb6c2' }, { core: '#ff6fde', glow: '#c23aa0' }]
    },
    paper: {
      label: 'Paper', sub: 'inverted — black on white',
      frameBg: '#ffffff', iconBg: '#ffffff', intr: null,
      hi: '#454c54', lite: { d: ['#2c3138', '#20242a'] }, drk: { d: ['#14171b', '#0e1013'] }, edge: '#08090b',
      collar: '#0b0d10', node: '#0b0d10',
      lanes: [{ core: '#0b0d10', glow: '#6b7682' }, { core: '#2c3138', glow: '#97a1ad' }]
    }
  };

  // Cross-section ramp (top->bottom) for thickness T: 3 lit wall rows,
  // T-6 interior channel rows, 3 shadow wall rows. Min T = 8.
  function buildRamp(th, T) {
    var r = [th.hi, th.lite, th.lite];
    for (var i = 0; i < T - 6; i++) r.push('IN');
    r.push(th.drk); r.push(th.drk); r.push(th.edge);
    return r;
  }

  function cfg(opts) {
    var t = (opts && opts.theme) || 'mono';
    t = typeof t === 'string' ? THEMES[t] : t;
    var lanes = Math.max(1, (opts && opts.lanes) || 2);
    var band = Math.max(8, (opts && opts.band) || 11);
    var maxBand = Math.floor((N - (lanes - 1) * G) / lanes);
    return {
      theme: t,
      orient: (opts && opts.orient) === 'v' ? 'v' : 'h',
      lanes: lanes,
      band: Math.min(band, maxBand),
      coupling: (opts && opts.coupling) || null,
      nodes: !!(opts && opts.nodes)
    };
  }

  function bands(lanes, T) {
    var total = lanes * T + (lanes - 1) * G, start = Math.floor((N - total) / 2), out = [];
    for (var i = 0; i < lanes; i++) out.push(start + i * (T + G));
    return out;
  }

  function toXY(orient, u, v) { return orient === 'h' ? [u, v] : [v, u]; }
  function resolve(entry, x, y) { return (entry && entry.d) ? entry.d[((x + y) % 2 + 2) % 2] : entry; }
  function inBounds(x, y) { return x >= 0 && x < N && y >= 0 && y < N; }

  function buildGrid(opts) {
    var c = cfg(opts), th = c.theme, T = c.band, tops = bands(c.lanes, T);
    var phase = (opts && opts.phase) || 0, withPackets = !opts || opts.packets !== false;
    var ramp = buildRamp(th, T);
    var grid = [], intr = [];
    for (var y = 0; y < N; y++) { grid.push(new Array(N).fill(null)); intr.push(new Array(N).fill(false)); }

    var laneIntr = [];
    for (var li = 0; li < c.lanes; li++) {
      var top = tops[li], iA = null, iB = null;
      for (var r = 0; r < T; r++) {
        var entry = ramp[r], cross = top + r, isIn = entry === 'IN';
        for (var u = 0; u < N; u++) {
          var xy = toXY(c.orient, u, cross), x = xy[0], y = xy[1];
          if (!inBounds(x, y)) continue;
          if (isIn) { grid[y][x] = th.intr; intr[y][x] = true; }
          else grid[y][x] = resolve(entry, x, y);
        }
        if (isIn) { if (iA === null) iA = cross; iB = cross; }
      }
      laneIntr.push([iA, iB]);
    }

    if (withPackets) {
      for (var l = 0; l < c.lanes; l++) {
        var range = laneIntr[l], c0 = range[0], c1 = range[1];
        var perLane = c.lanes === 1 ? 3 : 2, laneOff = l * 0.31;
        for (var p = 0; p < perLane; p++) {
          var f = (((phase + laneOff) + p / perLane) % 1 + 1) % 1;
          var u0 = Math.round(f * (N - 1));
          var col = c.lanes === 1 ? th.lanes[p % th.lanes.length] : th.lanes[l % th.lanes.length];
          paintRange(grid, intr, c.orient, u0 - 1, c0, c1, col.glow);
          paintRange(grid, intr, c.orient, u0 + COREW, c0, c1, col.glow);
          for (var w = 0; w < COREW; w++) paintRange(grid, intr, c.orient, u0 + w, c0, c1, col.core);
        }
      }
    }

    if (c.coupling) {
      for (var ci = 0; ci < c.coupling.length; ci++) {
        var u = Math.round(c.coupling[ci] * (N - 1));
        for (var ln = 0; ln < c.lanes; ln++) {
          var t0 = tops[ln];
          for (var rr = 0; rr < T; rr++) {
            var cr = t0 + rr;
            stamp(grid, c.orient, u - 1, cr, th.collar);
            stamp(grid, c.orient, u, cr, th.collar);
            stamp(grid, c.orient, u + 1, cr, resolve(th.drk, u + 1, cr));
          }
        }
      }
    }

    if (c.nodes && c.lanes >= 1) {
      var W = 5, clTop = tops[0], clBot = tops[c.lanes - 1] + T - 1;
      drawNode(grid, c.orient, 0, W - 1, clTop, clBot, th);
      drawNode(grid, c.orient, N - W, N - 1, clTop, clBot, th);
    }

    return { grid: grid, intr: intr };
  }

  function drawNode(grid, orient, u0, u1, c0, c1, th) {
    for (var u = u0; u <= u1; u++) {
      for (var cc = c0; cc <= c1; cc++) {
        var tone = (cc === c0 || u === u0) ? th.hi : (cc === c1 || u === u1) ? th.edge : resolve(th.lite, u, cc);
        stamp(grid, orient, u, cc, tone);
      }
    }
    var mu = Math.floor((u0 + u1) / 2), mc = Math.floor((c0 + c1) / 2);
    for (var a = 0; a < 2; a++) for (var b = 0; b < 2; b++) stamp(grid, orient, mu + a - 1, mc + b, th.node);
  }

  function stamp(grid, orient, u, v, color) {
    var xy = toXY(orient, u, v), x = xy[0], y = xy[1];
    if (inBounds(x, y)) grid[y][x] = color;
  }

  function paintRange(grid, intr, orient, u, c0, c1, color) {
    if (c0 === null) return;
    for (var cc = c0; cc <= c1; cc++) {
      var xy = toXY(orient, u, cc), x = xy[0], y = xy[1];
      if (!inBounds(x, y) || !intr[y][x]) continue;
      grid[y][x] = color;
    }
  }

  function draw(ctx, pxSize, opts) {
    opts = opts || {};
    var built = buildGrid(opts), grid = built.grid;
    if (opts.bg) { ctx.fillStyle = opts.bg; ctx.fillRect(0, 0, pxSize, pxSize); }
    else ctx.clearRect(0, 0, pxSize, pxSize);
    for (var y = 0; y < N; y++) {
      for (var x = 0; x < N; x++) {
        var k = grid[y][x];
        if (!k) continue;
        ctx.fillStyle = k;
        var x0 = Math.round(x * pxSize / N), x1 = Math.round((x + 1) * pxSize / N);
        var y0 = Math.round(y * pxSize / N), y1 = Math.round((y + 1) * pxSize / N);
        ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
      }
    }
  }

  root.ConduitLogo = { N: N, THEMES: THEMES, buildGrid: buildGrid, draw: draw };
})(typeof window !== 'undefined' ? window : this);
