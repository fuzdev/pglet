-- @setup
CREATE TABLE snok1 (id INT, v TEXT);
CREATE TABLE snok2 (id INT);
INSERT INTO snok1 VALUES (10, 'A'), (20, 'B'), (30, 'C');
INSERT INTO snok2 VALUES (10);

-- The read spelling of the write that forced the unary-minus fold. A subquery's
-- `ORDER BY -1` is a negative position (`42P10`) on both engines once the minus folds
-- into the constant — and before it did, the key was an ordinary constant sort and the
-- statement ran, which for the `DELETE`/`UPDATE` spellings of this same filter meant a
-- row removed or rewritten against an error. Unlike
-- `subquery_sort_key_binds_to_nothing_loud.sql`, the inner relation here **matches**, so
-- the shape needs no empty relation and no non-matching outer predicate: the silence was
-- row-count-independent. The row survival is
-- `crates/pglet/tests/scope_relation_identity.rs` — `@expect-error` compares SQLSTATEs
-- and is green for a refusal raised after the writes.
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
-- @test
SELECT v FROM snok1 WHERE id IN (SELECT id FROM snok2 ORDER BY -1);
