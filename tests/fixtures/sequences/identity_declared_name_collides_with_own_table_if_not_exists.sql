-- `IF NOT EXISTS` does not soften it. Its skip is tested at the top of parse analysis
-- and fires only when the table *already* exists; here the name is free until this
-- statement's own before-list takes it, so the heap creation still collides.
-- @test
CREATE TABLE IF NOT EXISTS idownine (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idownine));
-- @expect-error: relation already exists (42P07)
