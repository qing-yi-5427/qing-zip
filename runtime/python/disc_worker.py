"""Open Archive disc-container decoder. Independent bounded implementation.
ISZ layout: https://www.ezbsystems.com/isz/iszspec.txt
DAA 1.0 layout researched against Luigi Auriemma's daa2iso format reader.
Only single-volume unencrypted containers; writes one caller-selected ISO file.
"""
import bz2, pathlib, struct, sys, zlib
MAX_CHUNK=64*1024*1024
MAX_IMAGE=32*1024*1024*1024

def read(f,n):
    if n<0 or n>MAX_CHUNK: raise ValueError('invalid chunk size')
    b=f.read(n)
    if len(b)!=n: raise ValueError('truncated container')
    return b

def inflate(b,n,raw=False):
    d=zlib.decompressobj(-15 if raw else 15)
    out=d.decompress(b,n+1)
    if len(out)>n or not d.eof or d.unused_data or d.unconsumed_tail: raise ValueError('invalid compressed chunk')
    return out

def daa(f,out):
    h=read(f,76)
    if h[:16]!=b'DAA'+bytes(13):raise ValueError('invalid DAA signature')
    table,version,data,one,zero,chunk=struct.unpack_from('<6I',h,16)
    size,archive_size=struct.unpack_from('<QQ',h,40)
    if version!=0x100:raise ValueError('only DAA 1.0 supported')
    if zlib.crc32(h[:72])!=struct.unpack_from('<I',h,72)[0]:raise ValueError('DAA header CRC mismatch')
    if one!=1 or zero!=0 or not 0<chunk<=MAX_CHUNK or not 0<size<=MAX_IMAGE:raise ValueError('invalid DAA geometry')
    count=(size+chunk-1)//chunk
    if count>1000000 or data-table!=count*3 or table<76 or data>archive_size:raise ValueError('invalid DAA chunk table')
    f.seek(0,2)
    if f.tell()!=archive_size:raise ValueError('split or truncated DAA unsupported')
    f.seek(76)
    while f.tell()<table:
        pos=f.tell();kind,length=struct.unpack('<II',read(f,8))
        if length<8 or pos+length>table:raise ValueError('invalid DAA metadata')
        if kind==1:raise ValueError('split DAA unsupported')
        if kind==3:raise ValueError('encrypted DAA unsupported')
        f.seek(pos+length)
    sizes=read(f,count*3)
    f.seek(data);written=0
    for i in range(count):
        a,b,c=sizes[i*3:i*3+3];n=(a<<16)|(c<<8)|b
        block=inflate(read(f,n),chunk,True)
        expected=min(chunk,size-written)
        if len(block)<expected or (i<count-1 and len(block)!=expected):raise ValueError('DAA chunk length mismatch')
        out.write(block[:expected]);written+=expected
    if written!=size or f.tell()!=archive_size:raise ValueError('DAA image length mismatch')

def isz(f,out):
    h=read(f,48)
    if h[:4]!=b'IsZ!' or h[5]!=1 or not 48<=h[4]<=64:raise ValueError('unsupported ISZ header')
    h+=read(f,h[4]-48)
    sector=struct.unpack_from('<H',h,10)[0];sectors=struct.unpack_from('<I',h,12)[0]
    size=sector*sectors
    if h[16]:raise ValueError('encrypted ISZ unsupported')
    if struct.unpack_from('<Q',h,17)[0] or h[34] or struct.unpack_from('<I',h,39)[0]:raise ValueError('segmented ISZ unsupported')
    count,chunk=struct.unpack_from('<II',h,25);width=h[33]
    table=struct.unpack_from('<I',h,35)[0];data=struct.unpack_from('<I',h,43)[0]
    if sector==0 or not 0<chunk<=MAX_CHUNK or chunk%sector or not 0<size<=MAX_IMAGE or count!=(size+chunk-1)//chunk or count>1000000:raise ValueError('invalid ISZ geometry')
    if not 1<=width<=4 or table<len(h) or data<table+count*width:raise ValueError('invalid ISZ pointer table')
    f.seek(table);pointers=bytearray(read(f,count*width));key=b'\xb6\x8c\xa5\xde'
    for i in range(len(pointers)):pointers[i]^=key[i%4]
    f.seek(data);written=0;crc=0;packed_crc=0
    for i in range(count):
        value=int.from_bytes(pointers[i*width:(i+1)*width],'little');kind=value>>(width*8-2);n=value&((1<<(width*8-2))-1)
        if n>chunk:raise ValueError('invalid ISZ packed chunk length')
        expected=min(chunk,size-written)
        raw=read(f,n) if kind else b'';packed_crc=zlib.crc32(raw,packed_crc)
        if kind==0:block=bytes(expected)
        elif kind==1:block=raw
        elif kind==2:block=inflate(raw,chunk)
        else:
            if raw[:3]!=b'ISz':raise ValueError('invalid ISZ bzip2 signature')
            dec=bz2.BZ2Decompressor();block=dec.decompress(b'BZh'+raw[3:],max_length=chunk+1)
            if len(block)>chunk or not dec.eof or dec.unused_data:raise ValueError('invalid ISZ bzip2 chunk')
        if len(block)<expected or len(block)>chunk:raise ValueError('ISZ chunk length mismatch')
        block=block[:expected];out.write(block);crc=zlib.crc32(block,crc);written+=len(block)
    if written!=size:raise ValueError('ISZ output length mismatch')
    if len(h)==64:
        expected_crc=struct.unpack_from('<I',h,48)[0];expected_packed=struct.unpack_from('<I',h,60)[0]
        if expected_crc and expected_crc!=(crc^0xffffffff):raise ValueError('ISZ image CRC mismatch')
        if expected_packed and expected_packed!=(packed_crc^0xffffffff):raise ValueError('ISZ data CRC mismatch')

if __name__=='__main__':
    try:
        source,target=sys.argv[1:3]
        with open(source,'rb') as f,open(target,'xb') as out:
            (daa if pathlib.Path(source).suffix.lower()=='.daa' else isz)(f,out)
        print('OK')
    except Exception as e:
        print(str(e),file=sys.stderr);sys.exit(1)
