-- ...and the owner's *column* lookup outranks it too, one step further into
-- `process_owned_by` than the relation open.
-- @setup
CREATE TABLE aobmc_host (h INT);
CREATE TABLE aobmc_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobmc_t_id_seq OWNED BY aobmc_host.nosuch;
-- @expect-error: the missing owner column outranks the identity refusal (42703)
