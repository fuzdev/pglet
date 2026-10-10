-- referential_constraints carries each FK's delete_rule (from ON DELETE), with
-- update_rule from ON UPDATE (NO ACTION here, none being declared), match_option = NONE, and
-- unique_constraint_name resolved to the parent's PK/UNIQUE index.
-- @setup
CREATE TABLE rc_parent (id UUID PRIMARY KEY);
CREATE TABLE rc_child (
  id UUID PRIMARY KEY,
  a UUID REFERENCES rc_parent(id) ON DELETE CASCADE,
  b UUID REFERENCES rc_parent(id) ON DELETE SET NULL
);

-- @test
SELECT constraint_name::text AS constraint_name,
       unique_constraint_name::text AS unique_constraint_name,
       match_option::text AS match_option,
       update_rule::text AS update_rule,
       delete_rule::text AS delete_rule
FROM information_schema.referential_constraints
WHERE constraint_schema = 'public' AND constraint_name LIKE 'rc_child%'
ORDER BY constraint_name;
