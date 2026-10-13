# Reference sessions

Each file is one way an agent can build a benchmark task (project spec 23.2) through the MCP tools: a list of tool calls (`tool`, `args`) sent in order. `crates/mcp/tests/tasks.rs` replays them through the agent surface and checks that every call succeeds, every expectation passes, the meshes are closed and `diagnose` reports no errors or warnings.

| File | Task | Calls |
|---|---|---|
| `t1_barrel.json` | T1 wooden barrel with metal hoops (lathe, extruded hoop bands, recessed lid, weighted normals) | 2 |
| `t2_scifi_crate.json` | T2 sci-fi crate (recessed panels with raised plates, pipe handles unioned in, vent slots cut, beveled edges) | 2 |
| `t4_modular_wall.json` | T4 modular wall 4 m x 3 m with a doorway (pivot at the corner, mortar grooves cut, door frame, baseboards and cap unioned in) | 2 |
| `t5_pistol.json` | T5 first-person pistol (beveled slide and frame profiles, barrel, serrations, ejection port, stippled grip panels, sights, weighted normals) | 2 |

They cover geometry only: UVs, materials, bakes and export come with Phases 4 to 6.
