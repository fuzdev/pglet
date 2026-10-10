-- The referenced relation resolves first: a missing parent is `42P01` ahead of a referencing
-- column the new table lacks.
-- @expect-error: 42P01 relation nosuchtable does not exist
-- @test
CREATE TABLE ftc (a TEXT, FOREIGN KEY (nosuchcol) REFERENCES nosuchtable);
