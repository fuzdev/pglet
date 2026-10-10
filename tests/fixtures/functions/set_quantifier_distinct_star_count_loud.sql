-- The other half of the grammar bound: `func_name '(' '*' ')'` is its own production and
-- combines with no quantifier. This is the member that was silent over an arm that
-- legitimately takes `*` — pglet answered a count where 18.6 refuses the statement, and
-- `1` rather than the row count at that, since the `DISTINCT` deduplicated the wildcard
-- column it counts. 42601 on both.
-- @setup
CREATE TABLE sq_cds (a INT);
INSERT INTO sq_cds VALUES (1), (1), (2);
-- @test
-- @expect-error: syntax error at or near "*"
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the `*`, pglet at the closing paren
SELECT count(DISTINCT *) FROM sq_cds;
