-- The accept side of the INSERT target-column resolution: an unquoted column name in
-- the column list folds to the same key the catalog holds, so a mixed-case spelling
-- writes the column it names. The reject side — where the *catalog* name is quoted and
-- the list's is not — is `quoted_column_is_exact_loud.sql`; that pair is one rule, and
-- the fallback the reject side removes is exactly what used to make this side pass.
-- @setup
CREATE TABLE icfold (Val INT, other INT);

-- @test
INSERT INTO icfold (VAL, Other) VALUES (7, 8) RETURNING val, OTHER;
