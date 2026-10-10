-- **Which error wins, between a statement's clauses.** 18.6 analyses the whole target
-- list before the `WHERE`, so a target-list `unnest(<non-array>)` beside an unbindable
-- `WHERE` column is `42883` and not `42703`. pglet lowers the projection before the
-- predicate, so the same order falls out — but only once the projection *has* a gate
-- for this shape: before it the projection was silent and the `WHERE`'s `42703` was the
-- statement's answer.
-- @setup
CREATE TABLE unnest_na_prec (i INT);
-- @test
-- @expect-error: the target list's 42883 outranks the WHERE's 42703
SELECT unnest(i) FROM unnest_na_prec WHERE nosuchcol = 1;
