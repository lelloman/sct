CREATE TABLE meta(descriptor TEXT NOT NULL,snapshot TEXT NOT NULL);
            CREATE TABLE discs(id TEXT PRIMARY KEY,number INTEGER NOT NULL UNIQUE,set_sequence INTEGER NOT NULL);
            CREATE TABLE entries(path TEXT PRIMARY KEY,kind TEXT NOT NULL,root TEXT,version TEXT,length INTEGER NOT NULL);
            CREATE TABLE objects(root TEXT,digest TEXT,length INTEGER NOT NULL,PRIMARY KEY(root,digest),UNIQUE(root,digest,length));
            CREATE TABLE extents(path TEXT REFERENCES entries(path),offset INTEGER,root TEXT,digest TEXT,length INTEGER,PRIMARY KEY(path,offset),FOREIGN KEY(root,digest,length) REFERENCES objects(root,digest,length));
            CREATE TABLE placements(root TEXT,digest TEXT,length INTEGER,disc TEXT REFERENCES discs(id),PRIMARY KEY(root,digest,disc),FOREIGN KEY(root,digest,length) REFERENCES objects(root,digest,length));
            CREATE INDEX placement_disc ON placements(disc,root,digest);
