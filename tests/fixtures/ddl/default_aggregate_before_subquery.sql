-- Positional walk, aggregate/subquery pair, polarity 1 of 2. Both constructs are
-- forbidden in a DEFAULT and they carry different SQLSTATEs, so the arrangement
-- decides: the aggregate is positionally first here and 18.6 answers `42803`.
-- Mirror: `default_subquery_before_aggregate.sql` (`0A000`).
--
-- This pair discriminates a second rule beyond the child order — that a
-- subquery-bearing node is a verdict-at-node leaf rather than a ranked kind.
-- @expect-error: aggregate before the subquery wins (42803)
-- @test
CREATE TABLE dfe (a INT, b INT DEFAULT (count(*) + (SELECT 1)));
