-- An unquoted part of a name literal runs to the next `.` or whitespace, so a second
-- unquoted word is not a name at all: `SplitIdentifierString` fails and Postgres
-- answers `invalid name syntax` (42602). pglet read the whole literal as one name and
-- resolved the sequence, which is the accept-more half of the same defect that let
-- `'"a b"'` name the wrong relation.
-- @setup
CREATE SEQUENCE "s x";
-- @test
SELECT nextval('s x');
-- @expect-error: invalid name syntax (42602)
