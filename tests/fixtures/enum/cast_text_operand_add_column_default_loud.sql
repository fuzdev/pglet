-- `ADD COLUMN` evaluates its default once, for the rows it back-fills and those to come,
-- so the value conversion is raised at the `ALTER` — over an empty table too.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE u (id INT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE u ADD COLUMN m mood DEFAULT 'q'::text::mood;
