-- @setup
CREATE TABLE nseo (v INT);
CREATE TABLE nsei (w INT);
INSERT INTO nseo VALUES (1), (2);
INSERT INTO nsei VALUES (5), (6);

-- A target list is visible only to its **own** level's sort clauses, where a
-- `JOIN … USING` merge is a fact about a level's FROM and travels outward with it. That
-- is why `ColumnScope::output_names` is a sibling of `merged_names` rather than a second
-- spelling of it, and why `nested_in` shifts the one and not the other: the enclosing
-- query's `lbl` must not answer the inner `ORDER BY`.
--
-- The statement measured for that rule lived only inside the comment; this is it. Read
-- the guard narrowly, because that was measured too. The re-walk's output names are
-- pushed only for a key that *is* a bare `ColumnRef`
-- (`nested_sort_key_nested_output_name_loud.sql`), and such a key carries no sub-plan to
-- descend into, so no scope carrying output names reaches `nested_in` today: the
-- mutation that propagates them there reddens **nothing** on its own, and reddens this
-- fixture (`0A000` against 18.6's `42703`) only beside a revert of that shape gate. What
-- this pins is the answer, on both engines, however many mechanisms produce it.
-- @expect-error: an enclosing query's output alias does not answer a nested ORDER BY key
-- @test
SELECT v AS lbl FROM nseo ORDER BY (SELECT w FROM nsei ORDER BY lbl LIMIT 1);
