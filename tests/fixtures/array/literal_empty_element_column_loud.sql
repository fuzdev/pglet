-- The malformed literal is refused on its way into a column as well: nothing is stored.
-- @setup
CREATE TABLE array_literal_notes (id INT, tags TEXT[]);
-- @test
-- @expect-error: malformed array literal, 22P02
INSERT INTO array_literal_notes VALUES (1, '{a,,b}');
