# Layout quality gate

Checks the web studio's adaptive layout engine (`web/js/layout/`) with the real ELK (elkjs 0.9.3), independent of the engine's own metrics.

```bash
cd scripts/layout-gate
npm install
npm run gate                 # every input
npm run gate -- bundled      # only names containing "bundled" (comma-separate several filters)
npm run unit                 # overlap removal and packing property tests
npm run router               # orthogonal router suite
```

`OH_PROFILE=1` prints every layout candidate's time, score and metrics. `OH_DUMP_ROUTER=<dir>` writes each router call's input. `OH_OUT=<file>` sets the results file; the default is in the OS temp directory.

## Inputs (54)

- **`bundled/*`**: the 19 studio diagrams in `web/diagrams/`, validated as the studio validates them.
- **`code/*`**: class and package diagrams the in-browser extractor builds from `examples/`, `sample_codebase/`, `src/` and `web/js/`.
- **`synthetic/*` and `stress/*`**: larger, denser graphs. There is one stress input per diagram type, shaped like that type's bundled sample.

## Hard checks (a failure fails the gate)

- **Positions:** no node has a NaN position.
- **Overlaps:** no two sibling boxes overlap. A package's footprint includes its label band, and package boxes are emulated the way Cytoscape sizes compounds.
- **Containment:** children stay inside their package.
- **Edge ends:** every route starts and ends on its node border.
- **Routes:** orthogonal routes are axis-parallel, and no edge passes through an unrelated card or package.
- **Determinism:** two runs give identical output.

The edge checks skip three cases that the notation or Cytoscape handles. Sequence and timing messages cross lifelines by notation. Edges between a package and its own contents are drawn by Cytoscape. Hidden merged or embedded edges are not drawn at all.

## Soft metrics (reported, compared with the legacy layout)

Crossings, bends, label collisions, mean edge length in card sizes, area use, aspect ratio and run time.

## Last result (10 November 2026)

- **Hard checks:** all 54 inputs pass, and repeated runs give identical output.
- **Speed:** the two largest inputs, about 1,700 cards each, take about 21 s. Routing is about 55% of that.
- **Sprawl:** dense random stress graphs still have long edges (mean 7 to 11 card sizes).

See `CHECKPOINT.md`, section 0.
