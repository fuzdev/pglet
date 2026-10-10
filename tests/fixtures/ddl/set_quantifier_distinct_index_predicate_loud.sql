-- The fourth persisted-expression site — a partial index's predicate. Its pair with
-- `set_quantifier_distinct_index_expr_loud.sql` covers both index positions, which the
-- executor validates as two separate persisted-expression sites. 42809 on both.
-- @setup
CREATE TABLE sq_dixp (c TEXT);
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
CREATE INDEX sq_dixp_ix ON sq_dixp (c) WHERE upper(DISTINCT c) = 'A';
