-- A column carries **one** default. `transformColumnDefinition`'s `saw_default` makes
-- a second `DEFAULT` clause `42601`, and a last-wins assignment silently kept the
-- second and dropped the first.
-- @expect-error: multiple default values specified for column "a"
-- @test
CREATE TABLE def_multi (a INT DEFAULT 5 DEFAULT 7);
