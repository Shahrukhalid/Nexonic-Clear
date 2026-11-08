"""Catalog permitted dependency licenses and preserve upstream notices for packaging.

Only public registry packages and explicitly allowed SPDX licenses are accepted.
Exact crate source archives accompany MPL-2.0 packages.
No developer paths, credentials, environment values, or search data enter the catalog.
"""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET
import zipfile
import io

ALLOWED = {'MIT','MIT-0','Apache-2.0','BSD-2-Clause','BSD-3-Clause','ISC','Unicode-3.0','Unicode-DFS-2016','Zlib','MPL-2.0','CC0-1.0','Unlicense','BSL-1.0','OpenSSL','CDLA-Permissive-2.0','0BSD','Apache-2.0 WITH LLVM-exception'}
TOKENS = ALLOWED | {'AND','OR','WITH','LLVM-exception'}
ROOT = pathlib.Path(__file__).resolve().parents[1]


def license_ids(expression):
    if not expression:
        raise ValueError('A dependency has no declared license')
    ids = re.findall(r'[A-Za-z0-9][A-Za-z0-9.+:-]*', expression)
    unknown = set(ids) - TOKENS
    if unknown:
        raise ValueError(f'Unreviewed license expression: {expression}')
    return [value for value in ids if value not in {'AND','OR','WITH','LLVM-exception'}]


def copy_notices(base, destination):
    found = []
    for source in base.rglob('*'):
        if source.is_file() and source.name.upper().startswith(('LICENSE','LICENCE','COPYING','NOTICE','COPYRIGHT','AUTHORS')):
            target = destination / source.relative_to(base)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            found.append(str(target.relative_to(destination)))
    return found


def standard_text(license_id, output):
    path = output / 'standard-texts' / f'{license_id}.txt'
    if path.exists():
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    # Canonical public SPDX texts only; no arbitrary dependency-provided URL is fetched.
    subprocess.run(['curl','--fail','--silent','--show-error','--location',
                    f'https://raw.githubusercontent.com/spdx/license-list-data/main/text/{license_id}.txt','--output',str(path)], check=True)


def rust_notices(target, output):
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1','--filter-platform',target], cwd=ROOT))
    packages = {p['id']:p for p in metadata['packages']}
    nodes = {n['id']:n for n in metadata['resolve']['nodes']}
    reachable = set()
    stack = list(metadata['workspace_members'])
    while stack:
        package_id = stack.pop()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        stack.extend(nodes[package_id]['dependencies'])
    lock = tomllib.loads((ROOT/'Cargo.lock').read_text())
    checksums = {(p['name'],p['version']):p.get('checksum') for p in lock['package']}
    records = []
    for package_id in sorted(reachable):
        package = packages[package_id]
        if not package['source']:
            continue
        ids = license_ids(package['license'])
        base = pathlib.Path(package['manifest_path']).parent
        name, version = package['name'], package['version']
        directory = output/'rust'/f'{name}-{version}'
        found = copy_notices(base,directory)
        record = {'ecosystem':'Rust','name':name,'version':version,'license':package['license'],'notices':found,'repository':package.get('repository')}
        if not found:
            # No notice file shipped in the crate: record the declared copyright
            # holders and license; the standard license texts are bundled below.
            directory.mkdir(parents=True, exist_ok=True)
            authors=', '.join(package.get('authors') or []) or 'the authors of '+name
            (directory/'NOTICE.txt').write_text(f"{name} {version}\nCopyright (c) {authors}\nLicense: {package['license']}\nSource: {package.get('repository') or 'https://crates.io/crates/'+name}\n")
            found=['NOTICE.txt']
            record['notices']=found
        if 'MPL-2.0' in ids:
            # registry/src/<index>/<crate> -> registry/cache/<index>/<crate>.crate
            archive = base.parents[2]/'cache'/base.parent.name/f'{name}-{version}.crate'
            if not archive.is_file():
                raise ValueError(f'Exact source archive required for {name} {version}')
            expected = checksums.get((name,version))
            if not expected or hashlib.sha256(archive.read_bytes()).hexdigest()!=expected:
                raise ValueError(f'Source archive integrity failed for {name}')
            target_path=output/'source-archives'/archive.name
            target_path.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(archive,target_path)
            record['sourceArchive']='source-archives/'+archive.name
        for value in ids:
            standard_text(value,output)
        records.append(record)
    sysroot = pathlib.Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip())
    for name in ['COPYRIGHT','LICENSE-APACHE','LICENSE-MIT']:
        source=sysroot/'share/doc/rust'/name
        if source.is_file():
            destination=output/'rust-standard-library'/name
            destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(source,destination)
    return records


def javascript_notices(output):
    lock=json.loads((ROOT/'package-lock.json').read_text())
    records=[]
    for relative, entry in lock['packages'].items():
        if not relative or entry.get('dev'):
            continue
        base=ROOT/relative
        if not base.is_dir():
            if entry.get('optional'):
                continue
            raise ValueError(f'Runtime package missing: {relative}')
        info=json.loads((base/'package.json').read_text())
        declared=entry.get('license') or info.get('license')
        if isinstance(declared,dict):
            declared=declared.get('type')
        ids=license_ids(declared)
        name=info['name']
        found=copy_notices(base,output/'javascript'/name.replace('/','_'))
        if not found:
            raise ValueError(f'Original runtime notices missing: {name}')
        for value in ids:
            standard_text(value,output)
        records.append({'ecosystem':'JavaScript','name':name,'version':entry['version'],'license':declared,'notices':found})
    return records


def maven_notices(manifest, output):
    records=[]
    namespace={'m':'http://maven.apache.org/POM/4.0.0'}
    names={'The Apache Software License, Version 2.0':'Apache-2.0',
           'Apache License, Version 2.0':'Apache-2.0',
           'The Apache License, Version 2.0':'Apache-2.0',
           'Apache 2.0':'Apache-2.0', 'Apache-2.0':'Apache-2.0',
           'Apache License 2.0':'Apache-2.0', 'MIT License':'MIT',
           'The MIT License':'MIT', 'MIT':'MIT', 'BSD 3-Clause':'BSD-3-Clause'}
    def pom(group, name, version, destination, depth=0):
        if depth>5 or not all(re.fullmatch(r'[A-Za-z0-9_.-]+',v) for v in [group,name,version]):
            raise ValueError('Invalid Maven coordinates or excessive inheritance')
        relative=f"{group.replace('.', '/')}/{name}/{version}/{name}-{version}.pom"
        body=None
        destination.mkdir(parents=True,exist_ok=True)
        downloaded=destination/f'{name}-{version}.pom'
        for host in ['https://dl.google.com/dl/android/maven2/', 'https://repo.maven.apache.org/maven2/']:
            result=subprocess.run(['curl','--fail','--silent','--show-error','--proto','=https',
                '--max-time','30','--max-filesize','1048576','--write-out','%{http_code}',
                host+relative,'--output',str(downloaded)],capture_output=True,text=True)
            if result.returncode==0 and result.stdout=='200':
                body=downloaded.read_bytes()
                break
            if result.stdout!='404':
                raise ValueError(f'Maven metadata download failed: {group}:{name}:{version}')
        if body is None:
            raise ValueError(f'Maven POM missing: {relative}')
        root=ET.fromstring(body)
        declared=[x.findtext('m:name',namespaces=namespace) for x in root.findall('m:licenses/m:license',namespace)]
        if not declared:
            parent=root.find('m:parent',namespace)
            if parent is None:
                raise ValueError(f'No Maven license: {relative}')
            return pom(*(parent.findtext('m:'+key,namespaces=namespace) for key in ['groupId','artifactId','version']),destination,depth+1)
        ids=[]
        for value in declared:
            if value not in names:
                raise ValueError(f'Unreviewed Maven license: {name}: {value}')
            ids.extend(license_ids(names[value]))
        return ids
    for line in pathlib.Path(manifest).read_text().splitlines():
        group,name,version,artifact=line.split('\t')
        directory=output/'android'/f'{group}.{name}-{version}'
        ids=pom(group,name,version,directory)
        notices=[]
        def extract_notices(archive, prefix=''):
            for item in archive.infolist():
                if item.is_dir():
                    continue
                relative=pathlib.PurePosixPath(item.filename)
                if relative.is_absolute() or '..' in relative.parts:
                    raise ValueError('Unsafe dependency archive path')
                if relative.name.upper().startswith(('LICENSE','NOTICE','COPYRIGHT','COPYING')):
                    destination=directory/prefix/str(relative)
                    destination.parent.mkdir(parents=True,exist_ok=True)
                    destination.write_bytes(archive.read(item))
                    notices.append(str(destination.relative_to(directory)))
                elif item.filename=='classes.jar':
                    with zipfile.ZipFile(io.BytesIO(archive.read(item))) as nested:
                        extract_notices(nested,'classes')
        with zipfile.ZipFile(artifact) as archive:
            extract_notices(archive)
        for value in ids:
            standard_text(value,output)
        records.append({'ecosystem':'Android Maven','name':group+':'+name,'version':version,'license':' OR '.join(ids),'notices':notices,'pomPreserved':True})
    return records


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--target',required=True)
    parser.add_argument('--out',default='src-tauri/notices')
    parser.add_argument('--maven-manifest')
    args=parser.parse_args()
    output=ROOT/args.out
    output.mkdir(parents=True,exist_ok=True)
    records=rust_notices(args.target,output)+javascript_notices(output)
    if args.maven_manifest:
        records += maven_notices(args.maven_manifest,output)
    shutil.copyfile(ROOT/'LICENSE',output/'LICENSE-Nexonic-Clear.txt')
    shutil.copyfile(ROOT/'THIRD_PARTY_NOTICES.md',output/'THIRD_PARTY_NOTICES.md')
    (output/'INDEX.json').write_text(json.dumps({'target':args.target,'packages':records},indent=2)+'\n')
    print(f'License gate passed: {len(records)} packages cataloged; original notices and required source archives preserved.')


if __name__=='__main__':
    try:
        main()
    except (ValueError,subprocess.CalledProcessError) as error:
        print(str(error),file=sys.stderr)
        raise SystemExit(1)
