-- **A false-rejection control — do not trim it.** `LIMIT` / `OFFSET` bind to the whole
-- query and touch no target-list entry, so an `unknown` constant under one is still
-- `unknown` and the target column resolves it. Measured on 18.6: this statement, its
-- `OFFSET 0` and `LIMIT ALL` spellings, and `… FROM src ORDER BY n LIMIT 1` are all
-- accepted there.
--
-- It is the exact pair-mate of
-- `assignment_cast_select_sort_resolves_the_unknown_loud.sql`, which differs in nothing
-- but the clause: judging the whole source plan by its **immediate** node refused every
-- one of those five, because a `LIMIT` lowers to a `Plan::Limit` wrapping the
-- projection and the fallback reads `Plan::output_columns`, which substitutes `text`.
-- @setup
CREATE TABLE acsl (id INT, b BOOL);
-- @test
INSERT INTO acsl (b) SELECT 't' LIMIT 1 RETURNING b;
