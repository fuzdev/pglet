-- One non-boolean operand, and a connective above it. Before the resolver was one
-- object the two spellings disagreed: bare `WHERE a` matched no row and
-- `WHERE a AND true` matched it, the row's fate decided by whether an `AND` sat
-- above the operand.
-- @setup
CREATE TABLE bca (a INT);
INSERT INTO bca VALUES (9);
-- @expect-error: argument of AND must be type boolean
-- @test
SELECT count(*) FROM bca WHERE a AND true;
