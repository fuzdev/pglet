-- The cast spelling of `bare_non_member_add_column_default_loud`, back-filled the same.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE u (id INT);
INSERT INTO u VALUES (1);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE u ADD COLUMN m mood NOT NULL DEFAULT 'q'::mood;
