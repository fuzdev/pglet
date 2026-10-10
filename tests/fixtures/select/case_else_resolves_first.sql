-- `CASE` feeds its ELSE branch FIRST into common-type resolution — `parse_expr.c` does
-- `resultexprs = lcons(newc->defresult, resultexprs)` ("the default result is considered
-- the most significant type"). It is observable exactly when the two branch types are
-- mutually implicit-coercible, as `text` and `varchar` are: then the tie keeps whichever
-- came first, so swapping THEN and ELSE swaps the declared type between `text` (25) and
-- `character varying` (1043). Both spellings render the same characters, so the declared
-- column type is the only evidence.
-- @setup
CREATE TABLE ce (id INT, t TEXT, v VARCHAR(10));
INSERT INTO ce VALUES (1, 'aa', 'bb');
-- @test
SELECT id,
       CASE WHEN id = 1 THEN t ELSE v END AS then_text_else_varchar,
       CASE WHEN id = 1 THEN v ELSE t END AS then_varchar_else_text,
       CASE WHEN id = 1 THEN 1 ELSE 2::BIGINT END AS then_int_else_bigint
FROM ce ORDER BY id;
