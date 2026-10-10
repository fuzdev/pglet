-- And the gate a statement never reaches answers nothing: a target the catalog
-- lacks is `42P01` on both engines, wherever the gate would otherwise have sat.
-- @expect-error: relation does not exist
-- @test
DELETE FROM bcnosuchtable WHERE 1;
