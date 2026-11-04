import re
TOK=re.compile(r'\(|\)|"(?:[^"\\]|\\.)*"|[^\s()]+')
def parse(t):
    st=[[]]
    for m in TOK.finditer(t):
        s=m.group()
        if s=='(':
            st.append([])
        elif s==')':
            x=st.pop(); st[-1].append(x)
        else:
            st[-1].append(s[1:-1].replace('\\"','"') if s[0]=='"' else s)
    return st[0][0]
def find(n,k):
    return [c for c in n if isinstance(c,list) and c and c[0]==k]
def nets(path):
    t=parse(open(path).read())
    out={}
    for n in find(find(t,'nets')[0],'net'):
        name=find(n,'name')[0][1]
        out[name]=[(find(x,'ref')[0][1],find(x,'pin')[0][1],(find(x,'pinfunction') or [[0,'']])[0][1]) for x in find(n,'node')]
    return out
if __name__=='__main__':
    import sys
    for k,v in sorted(nets(sys.argv[1]).items()):
        print(k,' '.join(f'{r}.{p}'+(f'({f})' if f else '') for r,p,f in v))
