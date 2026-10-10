-- An index key converting a `varchar` column to an enum: `42P17`, ahead of the taken index
-- name and the `IF NOT EXISTS` skip, which `index_create` decides last.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ik (id INT, s VARCHAR(10));
CREATE INDEX ik_ix ON ik (id);

-- @profile-lock
-- @test
-- @expect-error: enum_in is not immutable
-- @expect-message
CREATE INDEX IF NOT EXISTS ik_ix ON ik ((lower(s)::mood));
