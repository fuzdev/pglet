-- A **bound `$N`** beside a computed branch, over the extended protocol — the spine's own
-- bound-SQL path.
--
-- A `$N` is of unknown type and takes its set's target: the coercion pass reads
-- `{$1, length('ab')}` at the `integer` its computed branch is typed at and casts the
-- parameter to it, so both declared-type walks read `integer` for the fold, the
-- derived table's `ColumnDesc` says so, and `sum(c)` answers 18.6's `2`.
--
-- Before the pass read a `$N` as unknown the fold was left as written and reached both
-- walks as a raw `Coalesce` over an `Expr::Parameter` — the plan is never mutated to
-- bind — where the executor walk's answer for it was `text`, and `sum(c)` was
-- `42883 function sum(text) does not exist`: a refusal, on the leg a bound-SQL driver
-- runs on. That walk's typeless `$N` leaf is still what a fold beside a branch the pass
-- does not type reaches (`coalesce_untyped_first_branch_declared_type.sql` has the
-- mechanism), and its other consequence there is invisible to this harness and is
-- recorded rather than pinned: over a **text-format** `Bind` the column-type-aware
-- encoder reads the same answer, so `SELECT COALESCE($1, date_trunc('day', ts))` bound
-- NULL sent `2024-01-15 00:00:00` under a correct `timestamptz` `RowDescription`.
-- `@param` binds through tokio-postgres, which requests **binary** results, and
-- `timestamp` and `timestamptz` share one 8-byte encoding — so that spelling is green on
-- the defect here, exactly as `coalesce_untyped_first_branch_timestamptz.sql`'s binary
-- leg was.
--
-- `$1` is described `int4` by the fold and by the `id = $1` disjunct alike; the
-- `$1 IS NULL` disjunct is what keeps the row.
-- @setup
CREATE TABLE bp (id INT);
INSERT INTO bp VALUES (1);

-- @param $1 int4 NULL
-- @test
SELECT sum(c) AS bound_null_sum
FROM (SELECT COALESCE($1, length('ab')) AS c FROM bp WHERE $1 IS NULL OR id = $1) s;
