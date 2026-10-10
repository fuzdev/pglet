-- An aggregate in a FROM-clause function's argument is refused, as 18.6 refuses it while
-- it transforms the argument: before the item's alias list is counted and before the
-- select list is read.
-- @expect-error: aggregate functions are not allowed in functions in FROM
-- @expect-message
-- @test
SELECT nosuch FROM unnest(ARRAY[count(*)]) AS u(x, y);
