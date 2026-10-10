-- Under a qualifier naming a schema no relation pglet models can be in, `ALTER TABLE IF
-- EXISTS` is 18.6's skip — the schema is missing, so the relation is — before any
-- analysis of the action: the column type that does not exist is never resolved. pglet
-- answered `42P01` for the qualified name. (The harness compares no notice; the host
-- tests pin it.)
-- @test
ALTER TABLE IF EXISTS nosuchschema_atie40.atie40 ADD COLUMN x nosuchtype_atie40;
