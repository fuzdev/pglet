-- The reverse polarity: a quoted list name does not answer to its folded spelling.
-- @expect-error: a quoted list name is verbatim
-- @test
WITH c("Xy") AS (SELECT 1) SELECT xy FROM c;
