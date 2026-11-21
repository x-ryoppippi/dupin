import { Footer } from '../components/Footer.tsx';
import { Head } from '../components/Head.tsx';
import { Nav } from '../components/Nav.tsx';
import { Final } from '../sections/Final.tsx';
import { Hero } from '../sections/Hero.tsx';
import { Install } from '../sections/Install.tsx';
import { Modes } from '../sections/Modes.tsx';
import { Names } from '../sections/Names.tsx';
import { OutOfTheWay } from '../sections/OutOfTheWay.tsx';

export function Landing() {
	return (
		<html lang="en" data-theme="dark">
			<Head />
			<body>
				<Nav />
				<main>
					<Hero />
					<Modes />
					<Names />
					<OutOfTheWay />
					<Install />
					<Final />
				</main>
				<Footer />
			</body>
		</html>
	);
}
