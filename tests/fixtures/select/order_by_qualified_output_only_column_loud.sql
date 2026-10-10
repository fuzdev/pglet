-- The `ORDER BY` allowance is for **bare** keys only, and this is what fixes that.
-- `findTargetlistEntrySQL92` takes the output-name branch for a single-field name alone,
-- so a *qualified* key must name a base column of the relation it qualifies:
-- `ORDER BY oqo.x` over `SELECT b AS x` is `42703` on 18.6 even though `x` is an output
-- column. pglet sorted by the alias.
--
-- Its accept polarity is the bare spelling `ORDER BY x`, green on both engines and
-- exercised by `select/order_by_output_alias_precedence.sql` among others — the pair is
-- what says the gate splits on the *spelling* rather than declining `ORDER BY` wholesale.
-- @setup
CREATE TABLE oqo (a INT, b TEXT);
INSERT INTO oqo VALUES (1, 'z');

-- @expect-error: column oqo.x does not exist (42703)
-- @test
SELECT b AS x FROM oqo ORDER BY oqo.x;
