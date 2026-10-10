-- **A false-rejection control — do not trim it.** `column_type_map_for` hands back an
-- empty map for a relation the catalog does not hold, so closing the action scope over one
-- would call every name it carries unbound and report `42703` — ahead of, and instead of,
-- the `42P01` for the table that both engines give. The scope therefore stays **open**
-- when the target is unknown.
--
-- **The spelling is the whole fixture**, and the obvious one does not work: a *bogus
-- qualifier* (`SET v = zzz.v`) is `42P01` under a closed scope too, since `zzz` names no
-- relation there either — so it is green whether or not the condition exists and proves
-- nothing. A **bare** name is the discriminator: the closed scope would say
-- `42703 column "anycol" does not exist` about a table that is not there. The
-- target-qualified and `excluded`-qualified spellings discriminate for the same reason,
-- both relations being registered with empty column maps.
--
-- Its reject neighbours are the closed-scope fixtures beside it; drop the
-- `table_def.is_some()` condition and they all stay green while this one alone reddens.
-- @setup
CREATE TABLE ocmt_anchor (n INT);

-- @expect-error: relation "ocmt_nosuch" does not exist (42P01)
-- @test
INSERT INTO ocmt_nosuch VALUES (1, 'a') ON CONFLICT (n) DO UPDATE SET v = anycol;
