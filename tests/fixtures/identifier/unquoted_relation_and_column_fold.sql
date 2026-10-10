-- An unquoted identifier is downcased where the parser reads it, as Postgres's
-- scanner does, so one relation and one column are named by every case spelling of
-- them. The three statements deliberately use three different spellings of each
-- name: without the fold the `INSERT` alone is `42P01` (the setup fails), and with a
-- fold applied to only one of the parser's name positions the `@test` is.
-- The quoted half — a `"…"` spelling is exact and does *not* find these — is
-- `quoted_relation_is_exact_loud.sql` and `quoted_column_is_exact_loud.sql`.
-- @setup
CREATE TABLE Foo (A INT);
INSERT INTO foo VALUES (1);

-- @test
SELECT a FROM FOO;
