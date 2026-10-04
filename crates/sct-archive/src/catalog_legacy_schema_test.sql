CREATE TABLE catalog_meta(format INTEGER NOT NULL CHECK(format=1)); INSERT INTO catalog_meta VALUES(1);
                CREATE TABLE catalog_sets(collection TEXT NOT NULL,id TEXT NOT NULL,sequence INTEGER NOT NULL,index_hash TEXT NOT NULL,bundle_hash TEXT NOT NULL,descriptor TEXT NOT NULL,PRIMARY KEY(collection,id),UNIQUE(collection,sequence));
                CREATE TABLE catalog_discs(collection TEXT NOT NULL,id TEXT NOT NULL,number INTEGER NOT NULL,set_sequence INTEGER NOT NULL,PRIMARY KEY(collection,id),UNIQUE(collection,number));
                CREATE TABLE catalog_scans(collection TEXT NOT NULL,disc TEXT NOT NULL,descriptor TEXT NOT NULL,PRIMARY KEY(collection,disc),FOREIGN KEY(collection,disc) REFERENCES catalog_discs(collection,id));
                CREATE TABLE catalog_records(collection TEXT NOT NULL,set_id TEXT NOT NULL,position INTEGER NOT NULL,line BLOB NOT NULL,PRIMARY KEY(collection,set_id,position),FOREIGN KEY(collection,set_id) REFERENCES catalog_sets(collection,id));
                CREATE TABLE catalog_placements(collection TEXT NOT NULL,set_id TEXT NOT NULL,disc TEXT NOT NULL,root TEXT NOT NULL,digest TEXT NOT NULL,length INTEGER NOT NULL,PRIMARY KEY(collection,set_id,root,digest,disc),FOREIGN KEY(collection,set_id) REFERENCES catalog_sets(collection,id),FOREIGN KEY(collection,disc) REFERENCES catalog_discs(collection,id));
                CREATE INDEX catalog_placement_disc ON catalog_placements(collection,disc,root,digest,length);
                CREATE INDEX catalog_object_length ON catalog_placements(collection,root,digest,length);
