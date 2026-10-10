-- The same refusal through the other half of `gram.y`'s `character:` production, and the
-- message names the **internal** type rather than the spelling: `varchar`, where the
-- `char` spelling names `char`. One `anychar_typmodin` with two wrappers passing two
-- names.
-- @expect-error: a varchar length below 1
-- @expect-message
-- @test
SELECT 'ab'::varchar(0) AS never_reached;
