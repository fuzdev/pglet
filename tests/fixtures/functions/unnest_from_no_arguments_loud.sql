-- A FROM-clause `unnest` over no array resolves no signature, rather than a relation of
-- no columns.
-- @test
-- @expect-error: function unnest() does not exist (42883)
-- @expect-message
SELECT * FROM unnest() AS u;
