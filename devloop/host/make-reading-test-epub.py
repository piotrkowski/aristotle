#!/usr/bin/env python3
"""Write the reading-interactions test book, "Próba czytania" (EPUB 3).

usage: make-reading-test-epub.py OUT.epub

A short Polish text (written for this test) with Greek and Latin words, an
EPUB 3 footnote (noteref to an aside in a notes file), a Wolne Lektury style
annotation link, and three chapters in the table of contents. The reader's
preview (`aristotle --preview`) and the device scenario
(`devloop/kindle/scenarios/reading.txt`) use it, so word positions are known.
"""
import sys
import zipfile

TITLE = "Próba czytania"

CH1 = [
    'Sokrates powtarzał, że dusza, po grecku <span lang="grc">ψυχή</span>, '
    'jest nieśmiertelna<a epub:type="noteref" href="notes.xhtml#fn1"><sup>1</sup></a>. '
    'Kto naprawdę oddaje się filozofii, ten przez całe życie ćwiczy się '
    'w umieraniu i nie lęka się tego, co przyjdzie.',
    'Rzeczywistość, którą widzimy oczami, przemija; trwałe jest tylko to, '
    'co poznaje rozum. Dlatego filozof szuka <span lang="grc">λόγου</span> '
    'rzeczy, a nie ich pozoru, i ceni <span lang="grc">ἀρετὴ</span> wyżej '
    'niż bogactwo<a class="anchor" id="anchor-2" href="notes.xhtml#annotation-2"><sup>2</sup></a>.',
    'Rzymianie mówili o tym krócej: <i lang="la">memento mori</i>, pamiętaj '
    'o śmierci. Nie jest to wezwanie do smutku, lecz do uwagi: kto pamięta, '
    'że czas jest krótki, ten nie trwoni go na rzeczy błahe.',
    'Uczniowie słuchali w milczeniu. Jeden z nich zapytał, skąd wiadomo, że '
    'dusza istniała, zanim się urodziliśmy. Sokrates odpowiedział pytaniem: '
    'czy uczenie się nie jest przypominaniem sobie tego, co dusza już kiedyś '
    'widziała?',
]
CH2 = [
    'Drugi dzień rozmowy zaczął się od sporu o równość. Dwa kije wydają się '
    'równe jednemu, a nierówne drugiemu; samej równości nigdy jednak nie '
    'widzieliśmy oczami. Skąd więc ją znamy?',
    'Jeśli znamy ją sprzed narodzin, to dusza istniała wcześniej. Jeśli zaś '
    'istniała wcześniej, łatwiej uwierzyć, że przetrwa i później.',
] * 3
CH3 = [
    'Wieczorem przyniesiono czarę. Sokrates wziął ją spokojnie, jakby chodziło '
    'o zwykły napój, i poprosił przyjaciół, by nie płakali.',
    'Ostatnie jego słowa dotyczyły długu wobec Asklepiosa. Tak skończył '
    'człowiek, którego znaliśmy jako najlepszego i najmądrzejszego.',
] * 3

XHTML = """<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="pl" xml:lang="pl">
<head><title>{title}</title></head>
<body>
{body}
</body>
</html>
"""


def chapter(title, paras):
    body = f"<h2>{title}</h2>\n" + "\n".join(f"<p>{p}</p>" for p in paras)
    return XHTML.format(title=title, body=body)


NOTES = XHTML.format(
    title="Przypisy",
    body=(
        '<h2>Przypisy</h2>\n'
        '<aside epub:type="footnote" id="fn1"><p>Zob. Platon, <i>Fedon</i> 105e: '
        'dusza nie przyjmuje śmierci, tak jak ogień nie przyjmuje zimna.</p></aside>\n'
        '<div id="annotation-2" class="annotation"><a href="ch1.xhtml#anchor-2">2</a>. '
        '<em>ἀρετή</em> — dzielność, doskonałość; u Platona przede wszystkim cnota '
        'duszy. [przypis edytorski]</div>'
    ),
)

NAV = XHTML.format(
    title="Spis treści",
    body=(
        '<nav epub:type="toc"><ol>'
        '<li><a href="ch1.xhtml">Rozdział pierwszy. O duszy</a></li>'
        '<li><a href="ch2.xhtml">Rozdział drugi. O równości</a></li>'
        '<li><a href="ch3.xhtml">Rozdział trzeci. Wieczór</a></li>'
        '</ol></nav>'
    ),
)

OPF = f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id" xml:lang="pl">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:uuid:5b0a7c3e-aristotle-proba-czytania</dc:identifier>
<dc:title>{TITLE}</dc:title>
<dc:creator>Aristotle</dc:creator>
<dc:language>pl</dc:language>
<meta property="dcterms:modified">2026-10-17T00:00:00Z</meta>
</metadata>
<manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
<item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/>
<item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/>
</manifest>
<spine>
<itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="ch3"/><itemref idref="notes"/>
</spine>
</package>
"""

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    with zipfile.ZipFile(sys.argv[1], "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip", zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", CONTAINER, zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/content.opf", OPF, zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/nav.xhtml", NAV, zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/ch1.xhtml", chapter("Rozdział pierwszy. O duszy", CH1), zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/ch2.xhtml", chapter("Rozdział drugi. O równości", CH2), zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/ch3.xhtml", chapter("Rozdział trzeci. Wieczór", CH3), zipfile.ZIP_DEFLATED)
        z.writestr("OEBPS/notes.xhtml", NOTES, zipfile.ZIP_DEFLATED)


if __name__ == "__main__":
    main()
