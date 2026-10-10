-- The declared name is the table's own, and the collision is the *table's*: the
-- before-list creates the sequence successfully, then `heap_create_with_catalog`
-- finds `idown` taken. So the gate cannot live in the naming loop — the catalog does
-- not hold the table under construction, and this statement is what makes that
-- observable. pglet accepted it and ended with one name in `tables` and `sequences`
-- at once, whose dump then dropped the table and all its rows in both engines.
-- @test
CREATE TABLE idown (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idown));
-- @expect-error: relation already exists (42P07)
