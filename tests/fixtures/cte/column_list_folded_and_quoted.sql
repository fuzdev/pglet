-- An unquoted list name folds where the lexer reads it; a `"…"`-quoted one is verbatim.
-- @test
WITH c(Xy, "Zw") AS (SELECT 1, 2) SELECT xy AS folded, "Zw" AS quoted FROM c;
