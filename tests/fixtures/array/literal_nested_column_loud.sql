-- A nested literal is refused on its way into a column: nothing is stored.
-- @setup
CREATE TABLE array_literal_nested (id INT, tags TEXT[]);
-- @test
-- @expect-error: pglet does not support multidimensional arrays (Postgres accepts this)
INSERT INTO array_literal_nested VALUES (1, '{{a,b}}');
