-- The foreign-key half of `create_table_interval_check_yields_to_taken_index_name.sql`:
-- a missing FK parent is `42P01` on both engines, ahead of the interval CHECK's
-- pglet-only render refusal.
--
-- Same shape and the same reason — `execute_create_table`'s CHECK loop runs entirely
-- ahead of the FK resolution, so before the render probe moved into its own
-- whole-statement pass this answered `0A000` here with the CHECK written first *and*
-- with it written last. One arrangement therefore pins it.
-- @expect-error: the FK's missing parent, ahead of the interval CHECK (42P01)
-- @test
CREATE TABLE iv_fkp (
  t TIMESTAMPTZ,
  n INT,
  CHECK (t > now() - interval '1 day'),
  FOREIGN KEY (n) REFERENCES iv_fkp_nosuch (q)
);
