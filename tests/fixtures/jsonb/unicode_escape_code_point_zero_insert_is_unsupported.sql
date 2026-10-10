-- The column-assignment route: an INSERT of the literal into a jsonb column is refused
-- with the same `22P05` rather than writing a NUL into the stored string.
-- @setup
CREATE TABLE j (v jsonb);
-- @test
-- @expect-error: code point zero on the write path
-- @expect-message
INSERT INTO j VALUES ('"a\u0000b"');
