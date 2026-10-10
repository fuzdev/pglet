-- A zero-length delimited identifier is refused by Postgres's **scanner**, in every
-- position, so it is `42601` and not a name error. pglet accepted it everywhere but
-- `DEALLOCATE` (which hand-rolled the check), and this position is the worst of them:
-- it registered a relation with an **empty name** in the catalog, with no error.
-- The refusal now lives in pglet's lexer, once, so every position agrees.
-- @test
-- @expect-error: zero-length delimited identifier
CREATE TABLE "" (a int);
