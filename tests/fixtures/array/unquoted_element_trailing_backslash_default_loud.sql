-- A default whose array literal ends in a backslash is refused where it is declared.
-- @test
-- @expect-error: a backslash ending the body of a default's int[] literal is 22P02
CREATE TABLE trailing_backslash_default (b int[] DEFAULT '{1\}');
