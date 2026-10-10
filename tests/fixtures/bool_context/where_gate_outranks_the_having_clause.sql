-- `transformStmt` reaches the `WHERE` one clause before the `HAVING`, and the
-- pair is written cross-code so the SQLSTATE alone discriminates: `@expect-message`
-- cannot serve here, `Error::Type`'s sentence carrying a `type error: ` prefix
-- 18.6 does not. A `HAVING` gate placed above the `WHERE` answers `42703`.
-- @setup
CREATE TABLE bcord7 (k INT);
-- @expect-error: argument of WHERE must be type boolean, not the HAVING item's 42703
-- @test
SELECT max(k) FROM bcord7 WHERE 1 HAVING nosuchcol;
