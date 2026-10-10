-- The serial sequences are created *before* the table and its indexes
-- (`transformCreateStmt` puts them in the before-list), so a declared constraint
-- name equal to the sequence's is 42P07 -- the sequence took it first.
-- @profile-lock
-- @test
CREATE TABLE seqfirst (a SERIAL, CONSTRAINT seqfirst_a_seq PRIMARY KEY (a));
-- @expect-error: relation already exists (42P07)
