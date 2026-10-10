-- `SELECT *` over a **zero-column relation** — the shape `EDGES.md` row 59 recorded
-- and this fixture closes. The relation is built by `ALTER TABLE … DROP COLUMN` down
-- to nothing, which both engines accept (the other route to one, a `RETURNING`-less
-- data-modifying CTE, 18.6 refuses outright — row 61 — so it cannot be a fixture).
--
-- The subject is that **a result set of zero columns is not the absence of a result
-- set**. 18.6 sends `RowDescription(fields=0)` plus one zero-column `DataRow` per row,
-- on both legs; pglet used to read its column *count* instead, so the simple leg sent
-- no descriptor and dropped the rows with it — two rows lost under a `SELECT 2` tag —
-- while the extended leg answered `NoData` to both `Describe`s and then sent the rows
-- anyway. Both engines now answer the same bytes on both legs.
--
-- What each leg of the harness actually pins here: the binary leg compares two
-- zero-column rows against the recorded `[{}, {}]`, and the text leg compares pglet's
-- simple-query reply against Postgres's live — which is the leg the row-loss bug was
-- on, since a dropped `RowDescription` there takes the `DataRow`s with it and the
-- recorded row count falls to zero. **What neither leg pins is the descriptor byte
-- itself** — the harness's simple probe records a zero-field `RowDescription` and a
-- missing one identically, and the binary leg decodes `DataRow`s that followed a
-- `NoData` as happily as ones that followed a `RowDescription(0)`, so it was green on
-- the pre-fix build. The `Describe` half and the aborted-block gate are pinned by
-- raw-frame tests in `crates/pglet_server`'s `handlers::tests` instead. The
-- declared-type comparison has nothing to say (neither side lists a column), so it
-- lands in the harness's `no_columns` bucket rather than asserting.
--
-- Sibling shapes measured with a raw v3 client and matching 18.6 for the same reason
-- (no fixture of their own — one `@test` per file): the same query over an *empty*
-- zero-column relation and under `LIMIT 0` (`RowDescription(0)` and no rows, not a
-- missing descriptor), a derived table and a CTE over it, `HAVING`, a cross join of two
-- such relations, `UNION`, `UNION ALL` and `EXCEPT`. What still diverges is row 63, the
-- two clauses 18.6 requires at least one column for: `DELETE FROM zc RETURNING *` is
-- `42601` there and a no-result-set success here, and `SELECT DISTINCT * FROM zc` is
-- `42601` there and a one-row zero-column result set here.
-- @setup
CREATE TABLE zc (x INT);
INSERT INTO zc VALUES (1), (2);
ALTER TABLE zc DROP COLUMN x;

-- @test
SELECT * FROM zc;
