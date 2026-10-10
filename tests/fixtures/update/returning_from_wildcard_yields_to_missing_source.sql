-- **Error precedence** over a `RETURNING *`, the missing-source member. Its siblings are
-- `returning_from_wildcard_yields_to_set_column.sql` (`42703`),
-- `..._to_set_value.sql` (`22P02`) and `..._to_where_column.sql` (`42703`).
--
-- The `RETURNING` clause is resolved and its wildcard expanded at **plan time**, and all
-- four of these errors are raised at **execute** time in pglet (measured with a raw v3
-- client separating `Parse`/`Describe` from `Bind`/`Execute`). So anything the expansion
-- raises outranks every one of them, and this file is the member where that is a live
-- risk rather than a hypothetical: the `FROM` names a relation that does not exist, so
-- the expansion is the one pass that has to cope with a source it cannot enumerate. It
-- registers nothing for it and expands the target's columns alone, leaving the source's
-- `42P01` — which both engines answer — to win.
--
-- A refusal standing in for *no* Postgres error used to sit at that expansion point
-- (`0A000`, because pglet could not project a source column at all), and it outranked
-- all four; at `Parse`, so a driver's `prepare()` failed on a statement 18.6 prepares.
--
-- **The control is `returning_from_qualified_wildcard_yields_to_missing_source.sql`**:
-- the same statement with `RETURNING <target>.*`, which answered `42P01` throughout.
-- Same build, same statement, only the wildcard spelling moving the answer.
--
-- The four are four independent guards over four distinct 18.6 SQLSTATEs raised at four
-- distinct sites in `execute_update` (`collect_updated_from`'s relation open,
-- `assignment_indices`, the `SET` coercion in the collect pass, the `WHERE` eval), so
-- deleting any one drops a site the others do not cover.
-- @setup
CREATE TABLE wpm_t (id INT, v TEXT);
INSERT INTO wpm_t VALUES (1, 'TARGET');

-- @test
-- @expect-error: 42P01 — the missing FROM source, on a statement whose RETURNING * spans it
UPDATE wpm_t SET id = 1 FROM wpm_nosuch s RETURNING *;
