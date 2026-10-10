-- The second normalization the same accept needs: an `AS` label lives on Postgres's
-- `TargetEntry` and not on the expression `equal()` compares, so `c AS c` and `c` are one
-- entry twice and 18.6 answers. Read it beside
-- `select/order_by_star_and_bare_duplicate_resolve.sql` (the qualified/bare half) and
-- `select/order_by_ambiguous_output_alias_loud.sql` (the refuse side).
-- @setup
CREATE TABLE sk_dup2 (c INT, x TEXT);
INSERT INTO sk_dup2 VALUES (1, 'a'), (2, 'b');

-- @test
SELECT c AS c, c FROM sk_dup2 WHERE false ORDER BY c;
